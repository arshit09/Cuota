//! Clawd on his own as a desktop pet (Show as: Desktop pet): no panel, just him, walking about
//! the screen in a window cut to his shape, so clicks around him reach whatever is behind.
//!
//! He does what Claude's doing. At work he works, now and then scuttling somewhere new; with
//! nothing on he sleeps, and every so often wakes, wanders off and sleeps somewhere else. With
//! React to pointer on, his eyes follow the pointer, and now and then he comes over to say hi,
//! keep it company or nap beside it. Resting the pointer on him, rather than passing over him,
//! shows a card of usage beside him. With Travel across monitors on, he walks over to the
//! monitors picked too, crossing where they meet.
//!
//! [`Roam`] decides where he goes and what he does, and [`Hover`] whether the pointer means to
//! look at him: both plain logic, tested on their own. [`Pet`] is his window, which the panel
//! keeps open by calling [`Pet::show`] each time it runs.

use crate::animations::{Act, BODY, Clawd, FRAME, Mood, Rng, SCALE};
use crate::claude::{Limit, State};
use crate::glance;
use crate::monitors::{self, Screen};
use crate::overlay::{BG, BORDER, CORNER, FRAME_TIME, GLANCE_GAP, MARGIN, MONITOR_CHECK, NOTE_WIDTH, note, repaint_after};
use crate::settings::{self, DoubleClick, View};
use crate::stats::Stats;
use crate::status::Status;
use crate::view::{self, TEXT};
use crate::widgets::Fonts;
use crate::win;
use eframe::egui::{
    Color32, Context, FontId, Galley, Painter, PointerButton, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2,
    ViewportBuilder, ViewportId, WindowLevel, pos2, vec2,
};
use std::collections::VecDeque;
use std::ops::Range;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::RECT;

/// His window's title, which is how it's found among this thread's windows.
const TITLE: &str = "Cuota pet";
/// Where his window opens, in points: off screen, until it has drawn him and can go where he is.
const OFF_SCREEN: Pos2 = pos2(-8000.0, -8000.0);
/// The size it opens at, in points, until he's laid out.
const FIRST_SIZE: Vec2 = vec2(300.0, 200.0);

/// He walks like a character in a pixel game, in hops rather than a glide: each one pixel of
/// his art across, down or both, which is `HOP` GIF pixels (an eye or a leg is one wide).
const HOP: f32 = 22.0;
/// How often he hops: at a scuttle, an amble, and a hurry over to the pointer. A hop is 6.6
/// points at his usual size, so straight across these come to 40, 22 and 55 points a second.
const WALK_HOP: Duration = Duration::from_millis(165);
const AMBLE_HOP: Duration = Duration::from_millis(300);
const VISIT_HOP: Duration = Duration::from_millis(120);
/// How far he wanders off at a time, in points...
const WANDER: Range<f32> = 80.0..320.0;
/// ...and at least how far he goes to sleep somewhere else.
const MOVE_AWAY: f32 = 200.0;
/// How long he does what his mood calls for before something else, in seconds: at work,
/// waiting, asleep, and in trouble.
const WORK_SPELL: Range<u64> = 15..45;
const WAIT_SPELL: Range<u64> = 10..25;
const SLEEP_SPELL: Range<u64> = 120..360;
const ERROR_SPELL: Range<u64> = 30..90;
/// Stopped on his way, or put down somewhere, he stays this long first.
const PAUSE: Range<u64> = 4..10;
/// Odds, as a spell ends, that he walks somewhere new while working, wanders off while
/// waiting, and (React to pointer on) heads for the pointer instead.
const WORK_WALK: f32 = 0.5;
const WANDER_ODDS: f32 = 0.5;
const VISIT_ODDS: f32 = 0.4;
/// Odds that a visit starts with a hello, rather than just hanging around (or, asleep, a nap).
const HELLO_ODDS: f32 = 0.6;
/// Travelling across monitors, odds that he goes to sleep on another one when he moves off.
const TRAVEL_ODDS: f32 = 0.5;
/// Beside the pointer, his feet are this far to its side and below it, in points.
const BESIDE: Vec2 = vec2(70.0, 40.0);
/// He gives up on reaching the pointer after this long.
const VISIT_TIME: Duration = Duration::from_secs(30);
/// He says it's your turn for this long at most, if you don't answer first.
const ASK_TIME: Duration = Duration::from_secs(120);
/// How long he keeps the pointer company, in seconds. He follows it once it has been more than
/// `FOLLOW` points from him for `FOLLOW_AFTER`.
const HANG: Range<u64> = 20..60;
const FOLLOW: f32 = 200.0;
const FOLLOW_AFTER: Duration = Duration::from_millis(1500);

/// The pointer rests on him once it's slower than this, in points a second...
const REST_SPEED: f32 = 60.0;
/// ...and his card shows once it has rested this long.
const REST: Duration = Duration::from_millis(450);
/// The pointer came to him itself, rather than him walking under it, if it moved this many
/// points in the last `MOVED_SPAN`.
const MOVED: f32 = 3.0;
const MOVED_SPAN: Duration = Duration::from_millis(300);
/// How far back the pointer's speed is measured.
const SPEED_SPAN: Duration = Duration::from_millis(80);
/// On him means within this many points of him.
const ON: f32 = 4.0;
/// The card stays this long after the pointer leaves him and it.
const LINGER: Duration = Duration::from_millis(400);

/// Room for his status under him, in points at his usual size, and from his feet to it.
const STATUS_ROOM: f32 = 200.0;
const STATUS_GAP: f32 = 2.0;
/// Around his status, so it reads over anything.
const RIM: Color32 = Color32::from_black_alpha(200);
/// Between him and his card, in points at his usual size.
const CARD_GAP: f32 = 10.0;
/// Between his eyes, in GIF pixels.
const EYES: Pos2 = pos2(235.0, 253.0);
/// How often to check for a full-screen app, which he goes behind.
const SCREEN_CHECK: Duration = Duration::from_secs(2);
/// How often to look at the pointer while it's close by or on its way to him, and otherwise
/// for his eyes.
const NEAR_TICK: Duration = Duration::from_millis(33);
const EYE_TICK: Duration = Duration::from_millis(150);
/// Close by is this many points from him.
const NEAR: f32 = 60.0;
/// While his card is hidden, it's measured this often, so his window keeps room for it.
const CARD_MEASURE: Duration = Duration::from_secs(1);
/// While his card shows Claude Code's stats, he asks the panel to read them again this often.
const STATS_ASK: Duration = Duration::from_secs(5);

pub fn viewport() -> ViewportId {
    ViewportId::from_hash_of("pet")
}

/// What [`Roam`] sees each frame. Positions are the top-left of his frame (see
/// [`crate::animations::FRAME`]), in pixels.
pub struct Senses {
    pub now: Instant,
    pub mood: Mood,
    /// The clip he was playing once, a greeting say, is over.
    pub played: bool,
    /// Where he is.
    pub at: Pos2,
    /// The pointer, when it's on his screen or one of `rooms`.
    pub pointer: Option<Pos2>,
    /// His screen, and what the taskbar leaves of it.
    pub area: Rect,
    pub work: Rect,
    /// The monitors he travels across that he can walk to from here, his own among them if
    /// it's one. Empty unless he travels.
    pub rooms: Vec<Room>,
    /// What has to stay on `work`, from `at`: him and his status.
    pub footprint: Rect,
    /// From `at` to his feet.
    pub feet: Vec2,
    /// Pixels to a point at his size: he walks faster and farther when bigger.
    pub scale: f32,
    /// React to pointer is on.
    pub react: bool,
    /// He stays where he is: looked at, carried, behind a full-screen app, or not animating.
    pub still: bool,
    /// Clicked, so he says hi.
    pub poked: bool,
    /// A session is waiting on you: a question, or a permission to give.
    pub asking: bool,
}

/// A monitor he can walk about: all of it, and what the taskbar leaves of it, in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Room {
    pub area: Rect,
    pub work: Rect,
}

/// Where two monitors meet, and he crosses from one to the other: a spot in the first with his
/// footprint at its edge, the spot in the second just across, and the band he keeps to between
/// them. Spots are his frame's top-left, as in [`Senses`].
#[derive(Clone, Copy, Debug, PartialEq)]
struct Doorway {
    near: Pos2,
    far: Pos2,
    band: Rect,
}

/// The way from `from` to `to`, when they meet along an edge with room for his `footprint`
/// along it on both, level with `at` as far as it can be.
fn doorway(from: Room, to: Room, at: Pos2, footprint: Rect) -> Option<Doorway> {
    // Side by side, or the same turned on its side for one above the other.
    let flip = |p: Pos2| pos2(p.y, p.x);
    let flip_rect = |r: Rect| Rect::from_min_max(flip(r.min), flip(r.max));
    let flip_room = |r: Room| Room { area: flip_rect(r.area), work: flip_rect(r.work) };
    let unflip = |door: Doorway| Doorway { near: flip(door.near), far: flip(door.far), band: flip_rect(door.band) };
    beside(from, to, at, footprint)
        .or_else(|| beside(flip_room(from), flip_room(to), flip(at), flip_rect(footprint)).map(unflip))
}

/// [`doorway`] for monitors side by side.
fn beside(from: Room, to: Room, at: Pos2, footprint: Rect) -> Option<Doorway> {
    let rightward = (from.area.right() - to.area.left()).abs() <= 1.0;
    if !rightward && (from.area.left() - to.area.right()).abs() > 1.0 {
        return None;
    }
    // Clear of either taskbar, which he crosses only when it's between them.
    let top = from.work.top().max(to.work.top());
    let bottom = from.work.bottom().min(to.work.bottom());
    if bottom - top < footprint.height() {
        return None;
    }
    let y = at.y.clamp(top - footprint.top(), bottom - footprint.bottom());
    let (near, far) = if rightward {
        (from.work.right() - footprint.right(), to.work.left() - footprint.left())
    } else {
        (from.work.left() - footprint.left(), to.work.right() - footprint.right())
    };
    let across = from.work.left().min(to.work.left())..=from.work.right().max(to.work.right());
    Some(Doorway { near: pos2(near, y), far: pos2(far, y), band: Rect::from_x_y_ranges(across, top..=bottom) })
}

/// The way from `here` to `to` through `rooms`, a monitor at a time, not counting `here`.
/// `None` when there isn't one.
fn route(here: Room, to: Room, rooms: &[Room], footprint: Rect) -> Option<Vec<Room>> {
    // Nearest first, each monitor with the one it's reached from.
    let mut reached: Vec<(Room, usize)> = vec![(here, 0)];
    let mut next = 0;
    while let Some(&(room, _)) = reached.get(next) {
        if room.area == to.area {
            let mut way = Vec::new();
            let mut step = next;
            while step != 0 {
                way.push(reached[step].0);
                step = reached[step].1;
            }
            way.reverse();
            return Some(way);
        }
        for &other in rooms {
            let new = !reached.iter().any(|(seen, _)| seen.area == other.area);
            if new && doorway(room, other, Pos2::ZERO, footprint).is_some() {
                reached.push((other, next));
            }
        }
        next += 1;
    }
    None
}

/// Which of `rooms` he can walk to from `here`, through the others.
pub fn reachable(here: Room, rooms: &[Room], footprint: Rect) -> Vec<Room> {
    rooms.iter().copied().filter(|&room| room.area == here.area || route(here, room, rooms, footprint).is_some()).collect()
}

/// Where he goes this frame, and what he plays.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plan {
    pub at: Pos2,
    pub act: Act,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Doing {
    /// What his mood calls for, where he is, until then.
    Stay(Instant),
    /// Walking there, then doing `then`.
    Walk { to: Toward, act: Act, then: Then, since: Instant },
    /// Playing `act` once, then doing `then`.
    Once { act: Act, then: Then },
    /// Keeping the pointer company until then, following it when it goes, since `away`.
    Hang { until: Instant, away: Option<Instant> },
    /// Telling you Claude's waiting on you, until then or until you've answered.
    Ask(Instant),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Toward {
    Spot(Pos2),
    /// Beside the pointer, wherever it goes.
    Pointer,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Then {
    /// What his mood calls for, for a spell.
    Settle,
    /// Say hi, then keep the pointer company (or, asleep, nap there).
    Hello,
    /// Keep the pointer company, until then if that's set already.
    Hang(Option<Instant>),
    /// Just woken: off to sleep somewhere else.
    Move,
    /// Tell you it's your turn.
    Ask,
}

/// A walk, a hop at a time.
#[derive(Clone, Copy, Debug)]
struct Hops {
    /// The walk these are for: when it began.
    since: Instant,
    /// Where it began, whose grid of art pixels he hops along...
    from: Pos2,
    /// ...and where he'd be by now if he glided, which he's always within half a hop of.
    glide: Pos2,
    /// When he hops next.
    next: Instant,
}

/// Decides where Clawd goes and what he does there.
pub struct Roam {
    rng: Rng,
    /// Where he is, as last planned.
    at: Option<Pos2>,
    doing: Doing,
    /// The mood he was last in.
    mood: Option<Mood>,
    /// The walk he's on, or was last on.
    hops: Option<Hops>,
    /// Claude was waiting on you last time he looked.
    asked: bool,
}

impl Roam {
    /// He says hi as he arrives.
    pub fn new(rng: Rng) -> Self {
        let doing = Doing::Once { act: Act::Hello, then: Then::Settle };
        Self { rng, at: None, doing, mood: None, hops: None, asked: false }
    }

    /// Whether he's on his way somewhere.
    pub fn walking(&self) -> bool {
        matches!(self.doing, Doing::Walk { .. })
    }

    /// When he next hops, on his way somewhere.
    pub fn next_hop(&self) -> Option<Instant> {
        let Doing::Walk { since, .. } = self.doing else { return None };
        self.hops.filter(|hops| hops.since == since).map(|hops| hops.next)
    }

    pub fn update(&mut self, s: &Senses) -> Plan {
        let now = s.now;
        // Put somewhere else, by a drag or by Windows, he stays there a while.
        let mut at = match self.at {
            Some(at) if (at - s.at).length() <= 2.0 => at,
            Some(_) => {
                self.doing = Doing::Stay(now + self.rng.seconds(PAUSE));
                s.at
            }
            None => s.at,
        };
        // Wherever he's put, and however big, he's all on the screen, status and all.
        if !self.walking() {
            at = clamp(at, s);
        }

        if self.mood != Some(s.mood) {
            // A rest keeps going through a doze; anything else gives way to the new mood.
            let resting = |mood: Mood| matches!(mood, Mood::Waiting | Mood::Asleep);
            let keep = self.mood.is_none_or(resting) && resting(s.mood) && self.with_pointer();
            if self.mood.is_some() && !keep {
                self.doing = Doing::Stay(now + self.spell(s.mood));
            }
            self.mood = Some(s.mood);
        }
        // `played` is about the clip playing now, so it's taken before a click starts another.
        if let Doing::Once { then, .. } = self.doing
            && s.played
        {
            self.doing = self.then(then, s);
        }
        if s.poked && !matches!(self.doing, Doing::Once { .. }) {
            self.doing = Doing::Once { act: Act::Hello, then: Then::Settle };
        }
        // Claude asking you something: he comes to tell you, or tells you from where he is,
        // until it's answered.
        if s.asking && !self.asked {
            self.doing = if s.react && s.pointer.is_some() {
                Doing::Walk { to: Toward::Pointer, act: Act::Visit, then: Then::Ask, since: now }
            } else {
                Doing::Ask(now + ASK_TIME)
            };
        } else if !s.asking
            && matches!(self.doing, Doing::Ask(_) | Doing::Walk { then: Then::Ask, .. })
        {
            self.doing = Doing::Stay(now + self.spell(s.mood));
        }
        self.asked = s.asking;
        if s.still {
            if self.walking() {
                self.doing = Doing::Stay(now + self.rng.seconds(PAUSE));
            }
        } else {
            self.doing = match self.doing {
                Doing::Stay(until) if now >= until => self.next(s, at),
                Doing::Ask(until) if now >= until => Doing::Stay(now + self.spell(s.mood)),
                Doing::Hang { until, .. } if now >= until => Doing::Stay(now + self.spell(s.mood)),
                Doing::Hang { until, away } => self.hang(until, away, s, at),
                // The pointer out of reach, he says what he came to say from where he got to.
                Doing::Walk { to: Toward::Pointer, then, since, .. } if s.pointer.is_none() || now - since >= VISIT_TIME => {
                    if then == Then::Ask { Doing::Ask(now + ASK_TIME) } else { Doing::Stay(now + self.spell(s.mood)) }
                }
                doing => doing,
            };
            if let Doing::Walk { to, act, then, since } = self.doing {
                let target = match (to, s.pointer) {
                    (Toward::Spot(spot), _) => spot,
                    (Toward::Pointer, Some(pointer)) => self.beside(pointer, at, s),
                    (Toward::Pointer, None) => at,
                };
                let every = match act {
                    Act::Amble => AMBLE_HOP,
                    Act::Visit => VISIT_HOP,
                    _ => WALK_HOP,
                };
                // His legs get going a beat before he does, which also keeps his first hop off
                // the frame that loads his walking clip.
                let mut hops = match self.hops {
                    Some(hops) if hops.since == since => hops,
                    _ => Hops { since, from: at, glide: at, next: now + every },
                };
                if now - hops.next >= every {
                    // A frame late by a beat or more, after a hitch such as a new clip loading,
                    // starts the beat over. A hop now would land just after the one that hitch
                    // held up.
                    hops.next = now + every;
                } else if now >= hops.next {
                    // A hop along whichever way is longer to go, and along the other as often as
                    // keeps him on his line there, like a line of pixels.
                    let hop = HOP * SCALE * s.scale;
                    let (aim, within, doorway) = waypoint(at, target, s);
                    let left = (aim - hops.glide) / hop;
                    let longer = left.x.abs().max(left.y.abs());
                    if longer > 1.0 {
                        hops.glide += left / longer * hop;
                        // On his grid, though at the screen's edge that can be just off it.
                        at = fit(hops.from + ((hops.glide - hops.from) / hop).round() * hop, within, s);
                    } else if doorway {
                        // On through it, on a grid from there.
                        at = aim;
                        (hops.from, hops.glide) = (aim, aim);
                    } else {
                        at = aim;
                        self.doing = self.then(then, s);
                    }
                    hops.next += every;
                }
                self.hops = Some(hops);
            }
        }
        self.at = Some(at);
        let act = match self.doing {
            Doing::Walk { act, .. } | Doing::Once { act, .. } => act,
            Doing::Ask(_) => Act::Ask,
            Doing::Stay(_) | Doing::Hang { .. } => Act::Mood,
        };
        Plan { at, act }
    }

    /// He's with the pointer, on his way to it, or telling you it's your turn.
    fn with_pointer(&self) -> bool {
        matches!(
            self.doing,
            Doing::Hang { .. }
                | Doing::Ask(_)
                | Doing::Walk { to: Toward::Pointer, .. }
                | Doing::Once { then: Then::Hang(_), .. }
        )
    }

    fn spell(&mut self, mood: Mood) -> Duration {
        self.rng.seconds(match mood {
            Mood::Working => WORK_SPELL,
            Mood::Waiting => WAIT_SPELL,
            Mood::Asleep => SLEEP_SPELL,
            Mood::Error => ERROR_SPELL,
        })
    }

    /// What he does once a spell of his mood is over.
    fn next(&mut self, s: &Senses, at: Pos2) -> Doing {
        let now = s.now;
        let visit = s.react && s.pointer.is_some();
        match s.mood {
            Mood::Working if self.rng.chance(WORK_WALK) => {
                let spot = self.wander(at, s);
                walk(spot, Act::Walk, now)
            }
            Mood::Waiting if visit && self.rng.chance(VISIT_ODDS) => self.visit(now),
            Mood::Waiting if self.rng.chance(WANDER_ODDS) => {
                let spot = self.wander(at, s);
                walk(spot, Act::Amble, now)
            }
            Mood::Asleep => Doing::Once { act: Act::Wake, then: Then::Move },
            mood => Doing::Stay(now + self.spell(mood)),
        }
    }

    /// What he does once he has done something.
    fn then(&mut self, then: Then, s: &Senses) -> Doing {
        let now = s.now;
        match then {
            Then::Settle => Doing::Stay(now + self.spell(s.mood)),
            Then::Hello => {
                let then = if s.mood == Mood::Asleep { Then::Settle } else { Then::Hang(None) };
                Doing::Once { act: Act::Hello, then }
            }
            Then::Hang(until) => Doing::Hang { until: until.unwrap_or_else(|| now + self.rng.seconds(HANG)), away: None },
            Then::Ask => Doing::Ask(now + ASK_TIME),
            Then::Move if s.react && s.pointer.is_some() && self.rng.chance(VISIT_ODDS) => self.visit(now),
            Then::Move => {
                let spot = self.far_spot(self.at.unwrap_or(s.at), s);
                walk(spot, Act::Amble, now)
            }
        }
    }

    /// Off to the pointer: to say hi first, or not.
    fn visit(&mut self, now: Instant) -> Doing {
        let then = if self.rng.chance(HELLO_ODDS) {
            Then::Hello
        } else if self.mood == Some(Mood::Asleep) {
            Then::Settle
        } else {
            Then::Hang(None)
        };
        Doing::Walk { to: Toward::Pointer, act: Act::Visit, then, since: now }
    }

    /// Keeping the pointer company: after it, once it has been away a moment.
    fn hang(&mut self, until: Instant, away: Option<Instant>, s: &Senses, at: Pos2) -> Doing {
        let Some(pointer) = s.pointer else { return Doing::Hang { until, away: None } };
        if (pointer - (at + s.feet)).length() <= FOLLOW * s.scale {
            return Doing::Hang { until, away: None };
        }
        let since = away.unwrap_or(s.now);
        if s.now - since >= FOLLOW_AFTER {
            Doing::Walk { to: Toward::Pointer, act: Act::Visit, then: Then::Hang(Some(until)), since: s.now }
        } else {
            Doing::Hang { until, away: Some(since) }
        }
    }

    /// Where he stands beside the pointer: on the side he's coming from, unless the edge of the
    /// screen would put him on top of it.
    fn beside(&self, pointer: Pos2, at: Pos2, s: &Senses) -> Pos2 {
        let side = if at.x + s.feet.x <= pointer.x { -1.0 } else { 1.0 };
        // On the pointer's screen, which can be another he walks to.
        let work = s.rooms.iter().find(|room| room.area.contains(pointer)).map_or(s.work, |room| room.work);
        let place = |side: f32| fit(pointer + vec2(side * BESIDE.x, BESIDE.y) * s.scale - s.feet, work, s);
        let spot = place(side);
        if s.footprint.translate(spot.to_vec2()).contains(pointer) { place(-side) } else { spot }
    }

    /// Somewhere a short walk away.
    fn wander(&mut self, at: Pos2, s: &Senses) -> Pos2 {
        let mut spot = at;
        for _ in 0..8 {
            let angle = self.rng.between(0.0..std::f32::consts::TAU);
            let distance = self.rng.between(WANDER) * s.scale;
            spot = clamp(at + Vec2::angled(angle) * distance, s);
            if (spot - at).length() >= WANDER.start * s.scale / 2.0 {
                break;
            }
        }
        spot
    }

    /// Anywhere on his screen, well away from where he is, or, now and then when he travels,
    /// anywhere on another. Always another when his own isn't one he travels across.
    fn far_spot(&mut self, at: Pos2, s: &Senses) -> Pos2 {
        let others: Vec<Room> = s.rooms.iter().copied().filter(|room| room.area != s.area).collect();
        let leaving = others.len() == s.rooms.len();
        if !others.is_empty() && (leaving || self.rng.chance(TRAVEL_ODDS)) {
            let room = others[self.rng.below(others.len())];
            let (low, high) = (fit(room.work.min, room.work, s), fit(room.work.max, room.work, s));
            let x = self.rng.between(low.x..high.x.max(low.x + 1.0));
            return pos2(x, self.rng.between(low.y..high.y.max(low.y + 1.0)));
        }
        let (low, high) = (clamp(s.work.min, s), clamp(s.work.max, s));
        for _ in 0..8 {
            let x = self.rng.between(low.x..high.x.max(low.x + 1.0));
            let spot = pos2(x, self.rng.between(low.y..high.y.max(low.y + 1.0)));
            if (spot - at).length() >= MOVE_AWAY * s.scale {
                return spot;
            }
        }
        self.wander(at, s)
    }
}

/// A walk to `spot`, then a spell of what his mood calls for there.
fn walk(spot: Pos2, act: Act, now: Instant) -> Doing {
    Doing::Walk { to: Toward::Spot(spot), act, then: Then::Settle, since: now }
}

/// `at`, moved as little as it takes for his footprint to be on the screen.
fn clamp(at: Pos2, s: &Senses) -> Pos2 {
    fit(at, s.work, s)
}

/// `at`, moved as little as it takes for his footprint to be within `within`.
fn fit(at: Pos2, within: Rect, s: &Senses) -> Pos2 {
    let low = within.min - s.footprint.min.to_vec2();
    let high = within.max - s.footprint.max.to_vec2();
    pos2(at.x.clamp(low.x, high.x.max(low.x)), at.y.clamp(low.y, high.y.max(low.y)))
}

/// Where he heads next on his way to `target`, what he keeps within on the way there, and
/// whether that's a doorway he goes on through: straight there on his own screen, or through
/// the doorway into the next monitor toward one he travels to. Somewhere he can't walk to, he
/// goes as near as his own screen takes him.
fn waypoint(at: Pos2, target: Pos2, s: &Senses) -> (Pos2, Rect, bool) {
    let middle = target + s.footprint.center().to_vec2();
    let here = Room { area: s.area, work: s.work };
    let goal = s.rooms.iter().copied().find(|room| room.area.contains(middle) && room.area != s.area);
    let next = goal.and_then(|goal| route(here, goal, &s.rooms, s.footprint)?.first().copied());
    let door = next.and_then(|next| doorway(here, next, at, s.footprint));
    // In the doorway already, he goes on across.
    if let Some(door) = door
        && Rect::from_two_pos(door.near, door.far).expand(0.5).contains(at)
    {
        return (door.far, door.band, true);
    }
    // Still stepping onto his screen from another, he gets all the way onto it before he
    // turns, so no part of him goes where neither monitor is.
    let on = clamp(at, s);
    if (on - at).length() > 0.5 {
        return (on, s.work.union(s.footprint.translate(at.to_vec2())), true);
    }
    match door {
        Some(door) => (door.near, s.work, true),
        None if s.area.contains(middle) => (target, s.work, false),
        None => (clamp(target, s), s.work, false),
    }
}

/// What the pointer's doing with him.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    /// Elsewhere, or only passing over him.
    Away,
    /// Resting on him, nearly long enough to show the card: he stops to let it.
    Coming,
    /// His card shows.
    Open,
}

/// Tells a pointer resting on him, to see his card, from one passing over him on its way
/// elsewhere, or him walking under one that's still.
#[derive(Default)]
pub struct Hover {
    /// Where the pointer was lately, and when, in pixels.
    trail: VecDeque<(Instant, Pos2)>,
    /// The pointer moved onto him, or on him, by itself.
    meant: bool,
    /// Since when the pointer has rested on him.
    resting: Option<Instant>,
    open: bool,
    /// Since when the pointer has been off him and his card, while it shows.
    left: Option<Instant>,
}

impl Hover {
    /// `body` is where he is and `card` where his card shows, if it does; `scale` is pixels per point.
    pub fn update(&mut self, now: Instant, pointer: Pos2, body: Rect, card: Option<Rect>, scale: f32) -> Look {
        self.trail.push_back((now, pointer));
        while self.trail.get(1).is_some_and(|&(at, _)| now - at >= MOVED_SPAN) {
            self.trail.pop_front();
        }
        let on = body.expand(ON * scale).contains(pointer);
        if self.open {
            let near = on || card.is_some_and(|card| card.expand(ON * scale).contains(pointer));
            if near {
                self.left = None;
            } else if now - *self.left.get_or_insert(now) >= LINGER {
                *self = Self { trail: std::mem::take(&mut self.trail), ..Self::default() };
                return Look::Away;
            }
            return Look::Open;
        }
        if !on {
            (self.meant, self.resting) = (false, None);
            return Look::Away;
        }
        let moved = self.trail.iter().map(|&(_, at)| (at - pointer).length()).fold(0.0, f32::max);
        self.meant |= moved > MOVED * scale;
        if !self.meant || self.speed(now) > REST_SPEED * scale {
            self.resting = None;
            return Look::Away;
        }
        if now - *self.resting.get_or_insert(now) >= REST {
            self.open(now);
            return Look::Open;
        }
        Look::Coming
    }

    /// Shows the card now, as a double-click asks, until the pointer leaves.
    pub fn open(&mut self, now: Instant) {
        (self.open, self.left, self.resting) = (true, None, Some(now));
    }

    /// How fast the pointer went lately, in pixels a second.
    fn speed(&self, now: Instant) -> f32 {
        let Some(&(_, to)) = self.trail.back() else { return 0.0 };
        let from = self.trail.iter().rev().find(|&&(at, _)| now - at >= SPEED_SPAN).or(self.trail.front());
        let Some(&(then, from)) = from else { return 0.0 };
        let seconds = (now - then).as_secs_f32();
        if seconds > 0.0 { (to - from).length() / seconds } else { 0.0 }
    }
}

/// Which way his eyes look at the pointer from `eyes`, both in pixels: art pixels across (up to
/// two, for a pointer well off to the side, when they stay level as in his own art) and down.
pub fn eyes(eyes: Pos2, pointer: Pos2, scale: f32) -> [i32; 2] {
    let off = (pointer - eyes) / scale;
    let distance = off.length();
    if distance < 16.0 {
        return [0, 0];
    }
    let (across, down) = (off.x / distance, off.y / distance);
    let x = if across.abs() > 0.8 && off.x.abs() > 160.0 {
        2
    } else if across.abs() > 0.35 {
        1
    } else {
        0
    };
    let y = if x == 2 {
        0
    } else if down > 0.5 {
        1
    } else if down < -0.5 {
        -1
    } else {
        0
    };
    [x * across.signum() as i32, y]
}

/// What the panel hands him each time it runs.
#[derive(Clone)]
pub struct Given {
    pub state: Arc<State>,
    pub stats: Option<Arc<Stats>>,
    pub pet: settings::Pet,
    pub animate: bool,
    /// The panel's opacity, which his card shares.
    pub opacity: f32,
    pub double_click: DoubleClick,
    pub custom_target: String,
}

impl Given {
    fn same(&self, other: &Self) -> bool {
        let stats = match (&self.stats, &other.stats) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (a, b) => a.is_none() && b.is_none(),
        };
        Arc::ptr_eq(&self.state, &other.state)
            && stats
            && self.pet == other.pet
            && self.animate == other.animate
            && self.opacity == other.opacity
            && self.double_click == other.double_click
            && self.custom_target == other.custom_target
    }
}

/// What he asks of the panel.
#[derive(Default)]
pub struct Asks {
    /// Right-clicked: open settings at his page, on his screen, or close them. Where he is, in
    /// pixels.
    pub settings: Option<Pos2>,
    /// Settled somewhere new, to start there next time: his frame's top-left, in pixels.
    pub spot: Option<[f32; 2]>,
    /// His card shows Claude Code's stats, which want reading again.
    pub stats: bool,
}

/// The card beside him, laid out at the origin.
struct Card {
    size: Vec2,
    meters: view::Meters,
    note: Option<Arc<Galley>>,
    glance: glance::Glance,
    margin: f32,
    note_gap: f32,
    glance_gap: f32,
}

/// Clawd's window, kept open by the panel.
pub struct Pet {
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    clawd: Clawd,
    status: Status,
    roam: Roam,
    hover: Hover,
    given: Given,
    fonts: Fonts,
    /// His native window, once found and set up.
    window: Option<isize>,
    /// The window's size, in points, as last asked for.
    room: Vec2,
    /// Where his frame sits in the window, in points, and in pixels as it was last placed.
    origin: Vec2,
    origin_px: Option<Vec2>,
    /// The biggest his card has been at his size, which the window keeps room for either side
    /// of him so neither he nor his window moves as its numbers change. Measured while it
    /// shows, and once a second while it doesn't.
    card_room: Vec2,
    card_measured: Option<Instant>,
    /// His size, whether the card's on, and what it shows, that `card_room` is for.
    card_room_for: Option<(f32, bool, View)>,
    /// How many frames have been drawn at `room`. He comes on screen after two, already drawn.
    drawn: u32,
    /// He's on screen, rather than off it while his window draws its first frames.
    placed: bool,
    /// Where he starts, in pixels: his frame's top-left.
    home: Pos2,
    /// What his window was last cut to, in pixels.
    shape: Vec<[i32; 4]>,
    /// The status line's left edge from his frame's, in points, gliding to stay centred under
    /// him as words change.
    status_left: Option<f32>,
    /// Where the status line and the card were last drawn, in points.
    status_rect: Option<Rect>,
    card_rect: Option<Rect>,
    /// Being carried: from the pointer to his frame's top-left, in pixels.
    held: Option<Vec2>,
    poked: bool,
    /// The monitors, as last listed, and when.
    screens: Vec<Screen>,
    screens_read: Option<Instant>,
    /// A full-screen app is running, as last checked then.
    full_screen: bool,
    screen_checked: Option<Instant>,
    /// Where he last settled, as told the panel.
    told: Option<Pos2>,
    stats_asked: Option<Instant>,
    asks: Asks,
}

impl Pet {
    pub fn new(ctx: &Context, fonts: &Fonts, given: Given) -> Self {
        let mut status = Status::new();
        status.outline = Some(RIM);
        let home = start(&given.pet);
        let inner = Inner {
            clawd: Clawd::roaming(ctx),
            status,
            roam: Roam::new(Rng::new()),
            hover: Hover::default(),
            given,
            fonts: fonts.clone(),
            window: None,
            room: FIRST_SIZE,
            origin: Vec2::ZERO,
            origin_px: None,
            card_room: Vec2::ZERO,
            card_measured: None,
            card_room_for: None,
            drawn: 0,
            placed: false,
            home,
            shape: Vec::new(),
            status_left: None,
            status_rect: None,
            card_rect: None,
            held: None,
            poked: false,
            screens: Vec::new(),
            screens_read: None,
            full_screen: false,
            screen_checked: None,
            told: Some(home),
            stats_asked: None,
            asks: Asks::default(),
        };
        Self { inner: Arc::new(Mutex::new(inner)) }
    }

    /// Keeps his window open, with what's `given` as of now. The window goes when the panel
    /// stops calling this, so call it each time the panel runs.
    pub fn show(&self, ctx: &Context, given: Given) {
        {
            let mut inner = self.inner.lock().unwrap();
            if !inner.given.same(&given) {
                inner.given = given;
                ctx.request_repaint_of(viewport());
            }
        }
        // Always the same, so egui never moves or resizes it: he does that himself. It never
        // takes the keyboard, even as it opens.
        let builder = ViewportBuilder::default()
            .with_title(TITLE)
            .with_active(false)
            .with_inner_size(FIRST_SIZE)
            .with_position(OFF_SCREEN)
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_taskbar(false)
            .with_window_level(WindowLevel::AlwaysOnTop);
        let inner = self.inner.clone();
        ctx.show_viewport_deferred(viewport(), builder, move |ui, _| inner.lock().unwrap().frame(ui));
    }

    /// What he has asked of the panel since it last looked.
    pub fn take_asks(&self) -> Asks {
        std::mem::take(&mut self.inner.lock().unwrap().asks)
    }
}

/// Where he starts: where he was left, if that's still on a screen, or else by the clock.
fn start(pet: &settings::Pet) -> Pos2 {
    let scale = win::system_scale();
    let feet = BODY.center_bottom().to_vec2() * SCALE * pet.size * scale;
    if let Some([x, y]) = pet.pos
        && win::on_screen((x + feet.x) as i32, (y + feet.y) as i32)
    {
        return pos2(x, y);
    }
    // Standing on the taskbar, with room under him for his status.
    let work = win::work_area(0, 0).map_or(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 720.0)), to_rect);
    let under = (STATUS_GAP + 20.0) * pet.size * scale;
    pos2(work.right() - 160.0 * pet.size * scale, work.bottom() - under) - feet
}

fn to_rect(rect: RECT) -> Rect {
    Rect::from_min_max(pos2(rect.left as f32, rect.top as f32), pos2(rect.right as f32, rect.bottom as f32))
}

/// Whole pixels covering `rect`, and one more all round, so his anti-aliased edges aren't cut.
fn pixels(rect: Rect) -> [i32; 4] {
    [
        rect.min.x.floor() as i32 - 1,
        rect.min.y.floor() as i32 - 1,
        rect.max.x.ceil() as i32 + 1,
        rect.max.y.ceil() as i32 + 1,
    ]
}

impl Inner {
    /// A frame of his window: where he goes, what he plays, and his status and card.
    fn frame(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let now = Instant::now();
        let Some(window) = self.window.or_else(|| win::own_window(TITLE)) else {
            repaint_after(&ctx, FRAME_TIME);
            return;
        };
        if self.window.is_none() {
            win::set_up_pet(window);
            self.window = Some(window);
        }
        let ppp = ctx.pixels_per_point();
        let given = self.given.clone();
        let pet = &given.pet;
        let (size, animate) = (pet.size, given.animate);
        let zoom = SCALE * size;

        // Where everything is, in pixels.
        let window_at = to_rect(win::window_rect(window)).min;
        // His frame is where it was put in his window, in pixels, though the scale has changed.
        let at = match self.origin_px {
            Some(origin) if self.placed => window_at + origin,
            _ => self.home,
        };
        let body = BODY * (zoom * ppp);
        // Him and the band under him that his status takes, which slides along it to stay on
        // the screen.
        let mut footprint = body;
        if let Some(status) = self.status_rect {
            let band = (status.top() - self.origin.y) * ppp..=(status.bottom() - self.origin.y) * ppp;
            footprint = footprint.union(Rect::from_x_y_ranges(body.x_range(), band));
        }
        // His monitor, and the ones he can walk to when he travels.
        if self.screens_read.is_none_or(|read| now - read >= MONITOR_CHECK) {
            self.screens = monitors::list();
            self.screens_read = Some(now);
        }
        let room = |screen: &Screen| Room { area: screen.area, work: screen.work };
        let middle = at + body.center().to_vec2();
        let here = monitors::at(&self.screens, middle).map_or_else(
            || {
                let work = win::work_area(middle.x as i32, middle.y as i32).map_or(Rect::EVERYTHING, to_rect);
                Room { area: work, work }
            },
            room,
        );
        let work = here.work;
        let rooms = if pet.travel {
            let picked: Vec<Room> = self.screens.iter().filter(|screen| pet.monitors.contains(&screen.name)).map(room).collect();
            reachable(here, &picked, footprint)
        } else {
            Vec::new()
        };
        let cursor = win::cursor();
        let pointer = pos2(cursor.x as f32, cursor.y as f32);
        let near = |screen: &Screen| screen.area == here.area || rooms.iter().any(|room| room.area == screen.area);
        let pointer_near = monitors::at(&self.screens, pointer).is_none_or(near);
        let body_on_screen = body.translate(at.to_vec2());
        let card_on_screen = self.card_rect.map(|card| (card * ppp).translate(window_at.to_vec2()));

        let look = if pet.hover && self.placed && self.held.is_none() {
            self.hover.update(now, pointer, body_on_screen, card_on_screen, ppp)
        } else {
            Look::Away
        };
        if self.screen_checked.is_none_or(|checked| now - checked >= SCREEN_CHECK) {
            self.screen_checked = Some(now);
            let middle = body_on_screen.center();
            let full = win::full_screen_at(middle.x as i32, middle.y as i32);
            if full != self.full_screen {
                self.full_screen = full;
                win::set_on_top(window, !full);
            }
        }
        if self.held.is_some() && !win::mouse_down() {
            self.held = None;
        }

        // Where he goes, and what he does.
        let state = given.state.clone();
        let senses = Senses {
            now,
            mood: self.clawd.feeling(&state),
            played: self.clawd.played(),
            at,
            pointer: pointer_near.then_some(pointer),
            area: here.area,
            work,
            rooms,
            footprint,
            feet: BODY.center_bottom().to_vec2() * zoom * ppp,
            scale: ppp * size,
            react: pet.react && animate,
            still: look != Look::Away || self.held.is_some() || self.full_screen || !animate || !self.placed,
            poked: std::mem::take(&mut self.poked),
            asking: state.sessions.iter().any(|session| session.waiting),
        };
        let plan = self.roam.update(&senses);
        let to = self.held.map_or(plan.at, |grip| pointer + grip);
        // Not on screen yet, he starts wherever that put him (inside the screen, say).
        if !self.placed {
            self.home = to;
        }
        self.clawd.update(&state, animate, plan.act);
        if pet.status {
            self.status.set(self.clawd.status().0, animate);
        }

        // His frame in the middle of the window, his status under it, and room either side
        // for the card, which goes wherever the screen has room for it.
        let painter = ui.painter().clone();
        let frame = FRAME * zoom;
        let status_size = if pet.status { self.status.size(&painter, size) } else { Vec2::ZERO };
        let status_top = BODY.bottom() * zoom + STATUS_GAP * size;
        // The card's room starts over at another size, or when what it shows changes.
        let card_for = |(at_size, on, view): &(f32, bool, View)| *at_size == size && *on == pet.hover && *view == pet.card;
        if !self.card_room_for.as_ref().is_some_and(card_for) {
            self.card_room = Vec2::ZERO;
            self.card_measured = None;
            self.card_room_for = Some((size, pet.hover, pet.card.clone()));
        }
        let measure = look == Look::Open || self.card_measured.is_none_or(|at| now - at >= CARD_MEASURE);
        let card = (pet.hover && measure).then(|| self.card(&painter, size));
        if let Some(card) = &card {
            self.card_room = self.card_room.max(card.size);
            self.card_measured = Some(now);
        }
        let side = if pet.hover { self.card_room.x + CARD_GAP * size } else { 0.0 };
        let block = vec2(frame.x.max(STATUS_ROOM * size), frame.y.max(status_top + status_size.y));
        let extra = if pet.hover { (self.card_room.y - BODY.height() * zoom).max(0.0) } else { 0.0 };
        let wanted = vec2(block.x + 2.0 * side, block.y + 2.0 * extra).ceil();
        if wanted != self.room {
            self.room = wanted;
            self.drawn = 0;
        }
        self.origin = vec2(side + (block.x - frame.x) / 2.0, extra);
        let size_px = (self.room * ppp).round();
        if (ui.max_rect().size() - self.room).length() < 1.0 {
            self.drawn += 1;
        }
        // On screen once he's drawn there, and from then on wherever he goes.
        let window_to = if self.placed || self.drawn >= 2 {
            self.placed = true;
            (to - self.origin * ppp).round()
        } else {
            (OFF_SCREEN * ppp).round()
        };
        if self.placed {
            self.origin_px = Some(to - window_to);
        }
        // His screen, in the window's points.
        let screen = Rect::from_min_max(((work.min - window_to) / ppp).to_pos2(), ((work.max - window_to) / ppp).to_pos2());
        let resize = (to_rect(win::window_rect(window)).size() - size_px).length() >= 1.0;
        if window_to != window_at || resize {
            let size = resize.then_some((size_px.x as i32, size_px.y as i32));
            win::place_window(window, window_to.x as i32, window_to.y as i32, size);
        }

        // Him, his eyes on the pointer if it's about.
        let origin = Pos2::ZERO + self.origin;
        let eyes_at = window_to + (self.origin + EYES.to_vec2() * zoom) * ppp;
        let looking = if senses.react && look == Look::Away { eyes(eyes_at, pointer, ppp) } else { [0, 0] };
        let mut next = self.clawd.paint_frame(&painter, origin, zoom, animate, looking);

        // What he's up to, centred under him, gliding over as the words change width.
        self.status_rect = None;
        let mut gliding = false;
        if pet.status {
            let target = BODY.center().x * zoom - status_size.x / 2.0;
            let left = match self.status_left {
                Some(left) if animate && (target - left).abs() > 0.5 => {
                    gliding = true;
                    left + (target - left) * 0.25
                }
                _ => target,
            };
            self.status_left = Some(left);
            // At the screen's edge, a long word slides along to stay on it.
            let left = (origin.x + left).clamp(screen.left(), (screen.right() - status_size.x).max(screen.left()));
            let line = Rect::from_min_size(pos2(left, origin.y + status_top), status_size);
            self.status_rect = Some(line);
            let lively = self.clawd.status().1;
            let status_next = self.status.paint(&painter, line.min, lively, animate, size);
            next = next.into_iter().chain(status_next).min();
        }

        // The card, beside him where the screen has room, level with him as far as it can be,
        // and clear of his status, which a long word makes wider than him.
        self.card_rect = None;
        if let Some(card) = card
            && look == Look::Open
        {
            let him = Rect::from_min_max(origin + BODY.min.to_vec2() * zoom, origin + BODY.max.to_vec2() * zoom);
            let beside = self.status_rect.map_or(him, |line| him.union(line));
            let gap = CARD_GAP * size;
            let right = beside.right() + gap;
            let left = beside.left() - gap - card.size.x;
            let x = if right + card.size.x <= screen.right() || left < screen.left() { right } else { left };
            let y = (him.center().y - card.size.y / 2.0)
                .clamp(screen.top(), (screen.bottom() - card.size.y).max(screen.top()))
                .clamp(0.0, (self.room.y - card.size.y).max(0.0));
            let rect = Rect::from_min_size(pos2(x, y), card.size);
            self.paint_card(&painter, card, rect, size);
            self.card_rect = Some(rect);
        }

        // Cut to what's drawn, so clicks anywhere else reach what's behind him.
        let zoomed = |r: &Rect| Rect::from_min_max(origin + r.min.to_vec2() * zoom, origin + r.max.to_vec2() * zoom);
        let mut shape: Vec<[i32; 4]> = self.clawd.outline().iter().map(|r| pixels(zoomed(r) * ppp)).collect();
        if let Some(line) = self.status_rect {
            // Centred under him in steps of 16 pixels either side, so the shape changes once or
            // twice a word rather than with every letter typed or step of the glide.
            let middle = origin.x + BODY.center().x * zoom;
            let half = (middle - line.left()).max(line.right() - middle);
            let half = (half * ppp / 16.0).ceil() * 16.0 / ppp;
            shape.push(pixels(Rect::from_x_y_ranges(middle - half..=middle + half, line.y_range()) * ppp));
        }
        if let Some(card) = self.card_rect {
            let px = card * ppp;
            let [left, top, right, bottom] = [px.min.x, px.min.y, px.max.x, px.max.y].map(|edge| edge.round() as i32);
            let rect = RECT { left, top, right, bottom };
            shape.extend(win::rounded(rect, CORNER * size * ppp).into_iter().map(|r| [r.left, r.top, r.right, r.bottom]));
        }
        if shape != self.shape {
            let rects: Vec<RECT> = shape.iter().map(|&[left, top, right, bottom]| RECT { left, top, right, bottom }).collect();
            win::shape_window(window, &rects);
            self.shape = shape;
        }

        // Dragged, he's carried; clicked, he says hi; double-clicked, he does what the panel
        // would; right-clicked, settings.
        let him = zoomed(&BODY);
        let grab = self.status_rect.map_or(him, |line| him.union(line));
        let response = ui.interact(grab, ui.id().with("pet"), Sense::click_and_drag());
        if response.drag_started_by(PointerButton::Primary) {
            self.held = Some(to - pointer);
        } else if response.clicked() {
            self.poked = true;
        }
        if response.double_clicked() {
            match given.double_click {
                DoubleClick::Details if pet.hover => self.hover.open(now),
                DoubleClick::Details => {}
                DoubleClick::Website => win::open_claude(false),
                DoubleClick::App => win::open_claude(true),
                DoubleClick::Custom if given.custom_target.is_empty() => self.asks.settings = Some(body_on_screen.center()),
                DoubleClick::Custom => win::open_target(&given.custom_target),
            }
        }
        let card = self.card_rect.map(|card| ui.interact(card, ui.id().with("card"), Sense::click()));
        if response.secondary_clicked() || card.is_some_and(|card| card.secondary_clicked()) {
            self.asks.settings = Some(body_on_screen.center());
        }

        // Settled somewhere new, he's there next time; his card's stats are kept fresh.
        let settled = self.placed && !self.roam.walking() && self.held.is_none();
        if settled && self.told.is_none_or(|told| (told - to).length() > 1.0) {
            self.told = Some(to);
            self.asks.spot = Some([to.x, to.y]);
        }
        let stats = self.card_rect.is_some() && pet.card.details.use_stats();
        if stats && self.stats_asked.is_none_or(|asked| now - asked >= STATS_ASK) {
            self.stats_asked = Some(now);
            self.asks.stats = true;
        }
        if self.asks.settings.is_some() || self.asks.spot.is_some() || self.asks.stats {
            ctx.request_repaint_of(ViewportId::ROOT);
        }

        // Again when he next changes, moves, or might see the pointer do something.
        let mut soon = |wait: Duration| next = Some(next.map_or(wait, |next| next.min(wait)));
        if self.held.is_some() || gliding || !self.placed {
            soon(FRAME_TIME);
        }
        if let Some(hop) = self.roam.next_hop() {
            soon(hop.saturating_duration_since(now));
        }
        if look != Look::Away || body_on_screen.expand(NEAR * ppp).contains(pointer) {
            soon(NEAR_TICK);
        }
        if senses.react {
            soon(EYE_TICK);
        }
        soon(SCREEN_CHECK);
        // Never sooner than a frame after this one began: each of those asks for its own next
        // change, and together they'd draw him a few milliseconds apart.
        if let Some(next) = next {
            repaint_after(&ctx, next.max(FRAME_TIME).saturating_sub(now.elapsed()));
        }
    }

    /// The card's meters, and the details picked for it, laid out at `scale`.
    fn card(&self, painter: &Painter, scale: f32) -> Card {
        let given = &self.given;
        let view = &given.pet.card;
        let now = chrono::Utc::now();
        let limits: Vec<&Limit> = given.state.limits.iter().filter(|limit| view.shows(&limit.label)).collect();
        let meters = view::Builder { painter, strong: &self.fonts.strong, view, now, scale }.build(&limits);
        let width = meters.size.x.max(NOTE_WIDTH * scale);
        let font = FontId::proportional(12.0 * scale);
        let note = note(&given.state).map(|(text, color)| painter.layout(text, font, color, width));
        let source = glance::Source { state: &given.state, stats: given.stats.as_deref(), limits: &limits, now };
        let glance = glance::Builder { painter, source, scale, width }.build(view.details.shown());
        let note_size = note.as_ref().map_or(Vec2::ZERO, |note| note.size());
        let note_gap = if note.is_some() && meters.size.y > 0.0 { 4.0 * scale } else { 0.0 };
        let head = meters.size.y + note_gap + note_size.y;
        let glance_gap = if glance.size.y > 0.0 && head > 0.0 { GLANCE_GAP * scale } else { 0.0 };
        let inner = vec2(meters.size.x.max(note_size.x).max(glance.size.x), head + glance_gap + glance.size.y);
        let margin = MARGIN * scale;
        let size = (inner + Vec2::splat(2.0 * margin)).round();
        Card { size, meters, note, glance, margin, note_gap, glance_gap }
    }

    fn paint_card(&self, painter: &Painter, card: Card, rect: Rect, scale: f32) {
        let alpha = (self.given.opacity * 255.0) as u8;
        painter.rect_filled(rect, CORNER * scale, Color32::from_rgba_unmultiplied(BG.r(), BG.g(), BG.b(), alpha));
        painter.rect_stroke(rect, CORNER * scale, Stroke::new(1.0, BORDER), StrokeKind::Inside);
        let at = rect.min + Vec2::splat(card.margin);
        let mut below = at.y + card.meters.size.y + card.note_gap;
        let _ = card.meters.paint(painter, at);
        if let Some(note) = card.note {
            let height = note.size().y;
            painter.galley(pos2(at.x, below), note, TEXT);
            below += height;
        }
        let _ = card.glance.paint(painter, pos2(at.x, below + card.glance_gap));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TICK: Duration = Duration::from_millis(50);
    /// A greeting, or waking up, takes this long to play.
    const ONCE: Duration = Duration::from_millis(2400);

    /// Clawd at his usual size on a 1920 x 1040 screen at 100%, run a frame at a time, his
    /// window following his plans to the nearest pixel as it does on screen.
    struct Sim {
        roam: Roam,
        s: Senses,
        acting: Duration,
        plans: Vec<Plan>,
        /// The monitors he travels across, which he finds himself on as he moves, as on
        /// screen. None unless he travels.
        monitors: Vec<Room>,
    }

    impl Sim {
        fn new(seed: u64, mood: Mood) -> Self {
            let s = Senses {
                now: Instant::now(),
                mood,
                played: false,
                at: pos2(800.0, 600.0),
                pointer: None,
                area: Rect::from_min_size(Pos2::ZERO, vec2(1920.0, 1080.0)),
                work: Rect::from_min_size(Pos2::ZERO, vec2(1920.0, 1040.0)),
                rooms: Vec::new(),
                footprint: BODY * SCALE,
                feet: BODY.center_bottom().to_vec2() * SCALE,
                scale: 1.0,
                react: false,
                still: false,
                poked: false,
                asking: false,
            };
            Self { roam: Roam::new(Rng::seeded(seed)), s, acting: Duration::ZERO, plans: Vec::new(), monitors: Vec::new() }
        }

        /// He travels across `monitors`, starting on the first.
        fn travelling(mut self, monitors: &[Room]) -> Self {
            self.monitors = monitors.to_vec();
            self.s.at = monitors[0].work.center() - self.s.footprint.center().to_vec2();
            self.find_monitor();
            self
        }

        /// As his window does: his monitor is the one his middle's on.
        fn find_monitor(&mut self) {
            let middle = self.s.at + self.s.footprint.center().to_vec2();
            let off = |room: &&Room| room.area.distance_sq_to_pos(middle);
            let here = *self.monitors.iter().min_by(|a, b| off(a).total_cmp(&off(b))).unwrap();
            (self.s.area, self.s.work) = (here.area, here.work);
            self.s.rooms = reachable(here, &self.monitors, self.s.footprint);
        }

        fn run(&mut self, time: Duration) {
            for _ in 0..time.as_millis() / TICK.as_millis() {
                let plan = self.roam.update(&self.s);
                let footprint = self.s.footprint.translate(plan.at.to_vec2());
                if self.monitors.is_empty() {
                    assert!(self.s.work.expand(0.5).contains_rect(footprint), "off the screen at {:?}", plan.at);
                } else {
                    // Crossing over, he can be on two at once, but no part of him is off them all.
                    let on = |corner: Pos2| self.monitors.iter().any(|room| room.area.expand(0.5).contains(corner));
                    let corners = [footprint.left_top(), footprint.right_top(), footprint.left_bottom(), footprint.right_bottom()];
                    assert!(corners.into_iter().all(on), "off the monitors at {:?}", plan.at);
                }
                self.acting = if matches!(plan.act, Act::Hello | Act::Wake) { self.acting + TICK } else { Duration::ZERO };
                self.s.played = self.acting >= ONCE;
                self.s.at = plan.at.round();
                self.s.now += TICK;
                self.s.poked = false;
                self.plans.push(plan);
                if !self.monitors.is_empty() {
                    self.find_monitor();
                }
            }
        }

        /// Which of his monitors his middle is on, as of his last plan.
        fn monitor(&self) -> usize {
            let middle = self.s.at + self.s.footprint.center().to_vec2();
            self.monitors.iter().position(|room| room.area.contains(middle)).expect("he's on a monitor")
        }

        fn feet(&self) -> Pos2 {
            self.s.at + self.s.feet
        }

        /// The walks so far: their act, and how far each went, from start to finish.
        fn walks(&self) -> Vec<(Act, f32)> {
            let walking = |act| matches!(act, Act::Walk | Act::Amble | Act::Visit);
            let mut walks: Vec<(Act, Pos2, Pos2)> = Vec::new();
            for pair in self.plans.windows(2) {
                let [last, plan] = [pair[0], pair[1]];
                // On his way, or arriving, when his last hop has him doing something else.
                if walking(last.act) && (plan.act == last.act || !walking(plan.act)) {
                    if let Some((_, _, to)) = walks.last_mut() {
                        *to = plan.at;
                    }
                } else if walking(plan.act) {
                    walks.push((plan.act, last.at, plan.at));
                }
            }
            walks.into_iter().map(|(act, from, to)| (act, (to - from).length())).collect()
        }
    }

    fn minutes(minutes: u64) -> Duration {
        Duration::from_secs(60 * minutes)
    }

    #[test]
    fn he_says_hi_as_he_arrives_then_does_what_his_mood_calls_for() {
        let mut sim = Sim::new(1, Mood::Asleep);
        sim.run(Duration::from_secs(1));
        assert!(sim.plans.iter().all(|plan| plan.act == Act::Hello && plan.at == pos2(800.0, 600.0)));
        sim.run(Duration::from_secs(3));
        assert_eq!(sim.plans.last().unwrap().act, Act::Mood);
    }

    #[test]
    fn asleep_he_wakes_now_and_then_and_moves_off_to_sleep_somewhere_else() {
        let mut sim = Sim::new(2, Mood::Asleep);
        sim.run(minutes(30));
        let walks = sim.walks();
        assert!(walks.len() >= 3, "{walks:?}");
        for (act, length) in &walks {
            assert_eq!(*act, Act::Amble, "he ambles while sleepy");
            assert!(*length >= MOVE_AWAY - 1.0, "only {length} away");
        }
        // Each move comes after waking up, and most of the time he lies still.
        let woken = sim.plans.windows(2).filter(|pair| pair[0].act != Act::Wake && pair[1].act == Act::Wake).count();
        assert!(woken >= walks.len(), "{woken} wakings for {} walks", walks.len());
        let still = sim.plans.windows(2).filter(|pair| pair[0].at == pair[1].at).count();
        assert!(still as f32 > 0.8 * sim.plans.len() as f32);
    }

    #[test]
    fn at_work_he_works_where_he_is_and_now_and_then_walks_a_little_way() {
        let mut sim = Sim::new(3, Mood::Working);
        sim.run(minutes(10));
        let walks = sim.walks();
        assert!(!walks.is_empty());
        for (act, length) in &walks {
            assert_eq!(*act, Act::Walk);
            assert!(*length <= WANDER.end + 1.0, "{length}");
        }
        assert!(!sim.plans.iter().any(|plan| matches!(plan.act, Act::Visit | Act::Wake | Act::Amble)));
        // Work comes first, even with the pointer about.
        sim.s.react = true;
        sim.s.pointer = Some(pos2(300.0, 300.0));
        sim.run(minutes(5));
        assert!(!sim.plans.iter().any(|plan| plan.act == Act::Visit));
    }

    #[test]
    fn reacting_to_the_pointer_he_comes_over_and_stays_beside_it() {
        let mut sim = Sim::new(4, Mood::Waiting);
        sim.s.react = true;
        let pointer = pos2(1300.0, 300.0);
        sim.s.pointer = Some(pointer);
        let mut arrived = None;
        for _ in 0..600 {
            sim.run(Duration::from_secs(1));
            let end = sim.plans.windows(2).position(|pair| pair[0].act == Act::Visit && pair[1].act != Act::Visit);
            if let Some(end) = end {
                arrived = Some(sim.plans[end + 1]);
                break;
            }
        }
        let arrived = arrived.expect("he never came over");
        let feet = arrived.at + sim.s.feet;
        assert!(((feet - pointer).length() - BESIDE.length()).abs() < 2.0, "his feet are at {feet:?}");
        assert!(!sim.s.footprint.translate(arrived.at.to_vec2()).contains(pointer), "he stands beside it, not on it");
        assert!(matches!(arrived.act, Act::Hello | Act::Mood));
    }

    #[test]
    fn keeping_the_pointer_company_he_follows_it_when_it_goes() {
        let mut sim = Sim::new(5, Mood::Waiting);
        sim.s.react = true;
        sim.s.pointer = Some(pos2(900.0, 500.0));
        sim.run(Duration::from_secs(3));
        sim.roam.doing = Doing::Hang { until: sim.s.now + minutes(2), away: None };
        sim.run(Duration::from_secs(1));
        assert_eq!(sim.plans.last().unwrap().act, Act::Mood);
        // Moved off a little way, it's still in his company.
        sim.s.pointer = Some(sim.feet() + vec2(120.0, 0.0));
        sim.run(Duration::from_secs(4));
        assert!(sim.plans.iter().rev().take(80).all(|plan| plan.act == Act::Mood));
        // Gone far, he follows once it has been gone a moment.
        let far = pos2(200.0, 200.0);
        sim.s.pointer = Some(far);
        sim.run(FOLLOW_AFTER - Duration::from_millis(200));
        assert_eq!(sim.plans.last().unwrap().act, Act::Mood, "not at once");
        sim.run(Duration::from_millis(400));
        assert_eq!(sim.plans.last().unwrap().act, Act::Visit);
        sim.run(Duration::from_secs(25));
        assert!(((sim.feet() - far).length() - BESIDE.length()).abs() < 3.0, "he's at {:?}", sim.feet());
        assert!(matches!(sim.roam.doing, Doing::Hang { .. }));
    }

    #[test]
    fn asleep_and_reacting_he_sometimes_naps_beside_the_pointer() {
        let pointer = pos2(400.0, 250.0);
        let napped = (10..20).any(|seed| {
            let mut sim = Sim::new(seed, Mood::Asleep);
            sim.s.react = true;
            sim.s.pointer = Some(pointer);
            sim.run(minutes(30));
            // Arrived beside it, and still there a minute later, asleep.
            let feet = sim.s.feet;
            sim.plans.windows(1200).any(|minute| {
                let first = minute[0];
                let beside = ((first.at + feet - pointer).length() - BESIDE.length()).abs() < 2.0;
                beside && minute.iter().all(|plan| plan.at == first.at && plan.act == Act::Mood)
            })
        });
        assert!(napped);
    }

    #[test]
    fn held_still_he_stops_where_he_is() {
        let mut sim = Sim::new(6, Mood::Waiting);
        sim.run(Duration::from_secs(3));
        sim.roam.doing = walk(pos2(100.0, 100.0), Act::Amble, sim.s.now);
        sim.run(Duration::from_secs(1));
        assert_eq!(sim.plans.last().unwrap().act, Act::Amble);
        sim.s.still = true;
        sim.run(Duration::from_millis(100));
        let stopped = sim.plans.last().unwrap().at;
        sim.run(Duration::from_secs(20));
        assert!(sim.plans.iter().rev().take(400).all(|plan| plan.at == stopped && plan.act == Act::Mood));
        // Let go, he carries on with his day after a pause.
        sim.s.still = false;
        sim.run(minutes(3));
        assert!(sim.plans.iter().rev().take(3000).any(|plan| plan.at != stopped));
    }

    #[test]
    fn put_down_somewhere_else_he_stays_there_a_while() {
        let mut sim = Sim::new(7, Mood::Working);
        sim.run(Duration::from_secs(3));
        sim.roam.doing = walk(pos2(100.0, 100.0), Act::Walk, sim.s.now);
        sim.run(Duration::from_secs(1));
        // Dropped somewhere else, as a drag leaves him.
        sim.s.at = pos2(1500.0, 200.0);
        sim.run(Duration::from_secs(PAUSE.start));
        assert!(sim.plans.iter().rev().take(80).all(|plan| plan.at == pos2(1500.0, 200.0) && plan.act == Act::Mood));
    }

    #[test]
    fn a_click_says_hi() {
        let mut sim = Sim::new(8, Mood::Working);
        sim.run(Duration::from_secs(4));
        sim.s.poked = true;
        sim.run(Duration::from_millis(100));
        assert_eq!(sim.plans.last().unwrap().act, Act::Hello);
        sim.run(ONCE + Duration::from_millis(100));
        assert_eq!(sim.plans.last().unwrap().act, Act::Mood);
    }

    #[test]
    fn a_click_just_as_a_clip_played_once_ends_still_says_hi() {
        let mut sim = Sim::new(11, Mood::Waiting);
        sim.run(Duration::from_secs(4));
        // Work's done: the celebration has just finished as he's clicked.
        sim.s.played = true;
        sim.s.poked = true;
        let plan = sim.roam.update(&sim.s);
        assert_eq!(plan.act, Act::Hello);
        sim.run(TICK);
        assert_eq!(sim.plans.last().unwrap().act, Act::Hello);
    }

    #[test]
    fn put_down_partly_off_the_screen_he_steps_back_onto_it() {
        let mut sim = Sim::new(12, Mood::Error);
        sim.run(Duration::from_secs(4));
        // Dropped with his feet and status over the taskbar, at the bottom right.
        sim.s.at = pos2(1880.0, 1010.0);
        sim.run(TICK);
        let at = sim.plans.last().unwrap().at;
        assert!(sim.s.work.expand(0.5).contains_rect(sim.s.footprint.translate(at.to_vec2())), "{at:?}");
        let edge = sim.s.work.max - sim.s.footprint.max.to_vec2();
        assert!((at - edge).length() < 0.5, "only as far as it takes: {at:?}");
    }

    #[test]
    fn claude_waiting_on_you_he_comes_over_to_say_its_your_turn() {
        let mut sim = Sim::new(13, Mood::Working);
        sim.s.react = true;
        let pointer = pos2(400.0, 300.0);
        sim.s.pointer = Some(pointer);
        sim.run(Duration::from_secs(5));
        sim.s.mood = Mood::Waiting;
        sim.s.asking = true;
        sim.run(TICK);
        assert_eq!(sim.plans.last().unwrap().act, Act::Visit);
        sim.run(Duration::from_secs(25));
        let plan = *sim.plans.last().unwrap();
        assert_eq!(plan.act, Act::Ask);
        assert!(((plan.at + sim.s.feet - pointer).length() - BESIDE.length()).abs() < 3.0);
        // Still saying so a minute on, then answered: back to his day.
        sim.run(Duration::from_secs(60));
        assert_eq!(sim.plans.last().unwrap().act, Act::Ask);
        sim.s.asking = false;
        sim.s.mood = Mood::Working;
        sim.run(TICK);
        assert_eq!(sim.plans.last().unwrap().act, Act::Mood);
        // Without React to pointer, he says it where he is, for two minutes at most.
        sim.s.react = false;
        sim.s.asking = true;
        sim.s.mood = Mood::Waiting;
        sim.run(TICK);
        let at = sim.plans.last().unwrap().at;
        assert_eq!(sim.plans.last().unwrap().act, Act::Ask);
        sim.run(ASK_TIME - Duration::from_secs(1));
        assert!(sim.plans.iter().rev().take(100).all(|plan| plan.act == Act::Ask && plan.at == at));
        sim.run(Duration::from_secs(2));
        assert_ne!(sim.plans.last().unwrap().act, Act::Ask);
    }

    #[test]
    fn work_starting_stops_a_wander_where_he_is() {
        let mut sim = Sim::new(9, Mood::Waiting);
        sim.run(Duration::from_secs(3));
        sim.roam.doing = walk(pos2(100.0, 100.0), Act::Amble, sim.s.now);
        sim.run(Duration::from_secs(1));
        sim.s.mood = Mood::Working;
        sim.run(Duration::from_millis(100));
        let at = sim.plans.last().unwrap().at;
        sim.run(Duration::from_secs(WORK_SPELL.start - 1));
        assert!(sim.plans.iter().rev().take(200).all(|plan| plan.at == at && plan.act == Act::Mood));
    }

    #[test]
    fn he_walks_like_a_pixel_game_character_a_hop_at_a_time() {
        let mut sim = Sim::new(14, Mood::Working);
        sim.run(Duration::from_secs(3));
        let start = sim.plans.len();
        let spot = pos2(1300.0, 380.0);
        sim.roam.doing = walk(spot, Act::Walk, sim.s.now);
        sim.run(Duration::from_secs(20));
        let plans = &sim.plans[start..];
        assert_eq!(plans.last().unwrap().at, spot, "he gets there");
        // Each time he moved: on which frame, and how far.
        let hops: Vec<(usize, Vec2)> = plans
            .windows(2)
            .enumerate()
            .filter(|(_, pair)| pair[1].at != pair[0].at)
            .map(|(frame, pair)| (frame, pair[1].at - pair[0].at))
            .collect();
        assert!(hops.len() > 50, "{} hops", hops.len());
        let hop = HOP * SCALE;
        let (last, on_the_way) = hops.split_last().unwrap();
        for &(_, step) in on_the_way {
            // A pixel of his art the longer way, and none or one the other.
            let (long, short) = (step.x.abs().max(step.y.abs()), step.x.abs().min(step.y.abs()));
            assert!((long - hop).abs() < 0.01, "{step:?}");
            assert!(short < 0.01 || (short - hop).abs() < 0.01, "{step:?}");
        }
        assert!(on_the_way.iter().any(|(_, step)| step.x != 0.0 && step.y != 0.0), "now and then up as well as across");
        assert!(last.1.x.abs().max(last.1.y.abs()) <= hop + 0.01, "at most a hop onto the spot: {:?}", last.1);
        // On a beat, not every frame.
        for pair in hops.windows(2) {
            let gap = TICK * (pair[1].0 - pair[0].0) as u32;
            assert!(gap + TICK > WALK_HOP, "{gap:?}");
        }
    }

    #[test]
    fn after_a_hitch_he_doesnt_hop_twice_in_a_row() {
        let mut sim = Sim::new(15, Mood::Working);
        sim.run(Duration::from_secs(3));
        sim.roam.doing = walk(pos2(100.0, 100.0), Act::Walk, sim.s.now);
        sim.run(Duration::from_secs(1));
        // A frame held up half a second, as a clip loading can.
        sim.s.now += Duration::from_millis(500);
        let (before, late) = (sim.plans.last().unwrap().at, sim.plans.len());
        sim.run(TICK * 4);
        assert!(sim.plans[late..].iter().all(|plan| plan.at == before && plan.act == Act::Walk), "a beat first");
        sim.run(TICK);
        assert_ne!(sim.plans.last().unwrap().at, before, "then on he goes");
    }

    #[test]
    fn beside_the_pointer_at_the_screens_edge_he_takes_the_other_side() {
        let mut sim = Sim::new(10, Mood::Waiting);
        sim.run(Duration::from_secs(1));
        let s = &sim.s;
        // Coming from the left, he'd stand to its left, but there's no room there.
        let pointer = pos2(20.0, 500.0);
        let spot = sim.roam.beside(pointer, pos2(0.0, 500.0), s);
        assert!(spot.x + s.feet.x > pointer.x);
        assert!(!s.footprint.translate(spot.to_vec2()).contains(pointer));
    }

    /// A monitor at `min`, `size` big, with a taskbar along its bottom when `taskbar`.
    fn room(min: Pos2, size: Vec2, taskbar: bool) -> Room {
        let area = Rect::from_min_size(min, size);
        let work = if taskbar { Rect::from_min_max(area.min, area.max - vec2(0.0, 40.0)) } else { area };
        Room { area, work }
    }

    /// The main monitor, with its taskbar, and a smaller one lower down on its right.
    fn two_monitors() -> [Room; 2] {
        [room(Pos2::ZERO, vec2(1920.0, 1080.0), true), room(pos2(1920.0, 300.0), vec2(1280.0, 1024.0), false)]
    }

    #[test]
    fn monitors_that_meet_with_room_for_him_join_and_he_finds_his_way_through_them() {
        let footprint = BODY * SCALE;
        let [main, right] = two_monitors();
        // Above it, one that meets it only past its taskbar, and one off on its own.
        let above = room(pos2(200.0, -1080.0), vec2(1920.0, 1080.0), false);
        let corner = room(pos2(-1280.0, 1060.0), vec2(1280.0, 1024.0), false);
        let apart = room(pos2(4000.0, 0.0), vec2(1920.0, 1080.0), false);
        let all = [main, right, above, corner, apart];
        assert_eq!(reachable(main, &all, footprint), [main, right, above]);
        assert_eq!(route(above, right, &all, footprint), Some(vec![main, right]));
        assert_eq!(route(main, main, &all, footprint), Some(vec![]));
        // Picked without the one between, the far one is out of reach.
        assert_eq!(reachable(above, &[above, right], footprint), [above]);

        // Into the one on the right, level with him...
        let door = doorway(main, right, pos2(1000.0, 500.0), footprint).unwrap();
        assert_eq!(door.near, pos2(1920.0 - footprint.right(), 500.0));
        assert_eq!(door.far, pos2(1920.0 - footprint.left(), 500.0));
        // ...or, higher up than it goes, as near as can be.
        assert_eq!(doorway(main, right, pos2(1000.0, 0.0), footprint).unwrap().near.y, 300.0 - footprint.top());
        // Back, and up through the top.
        assert_eq!(doorway(right, main, pos2(2500.0, 500.0), footprint).unwrap().far.x, 1920.0 - footprint.right());
        let up = doorway(main, above, pos2(1000.0, 500.0), footprint).unwrap();
        assert_eq!((up.near, up.far), (pos2(1000.0, -footprint.top()), pos2(1000.0, -footprint.bottom())));
        assert_eq!(doorway(main, corner, Pos2::ZERO, footprint), None);
    }

    #[test]
    fn travelling_he_sleeps_on_either_monitor_and_no_part_of_him_is_ever_off_them() {
        let mut sim = Sim::new(21, Mood::Asleep).travelling(&two_monitors());
        let mut napped = [Duration::ZERO; 2];
        for _ in 0..90 * 6 {
            sim.run(Duration::from_secs(10));
            if sim.plans.last().unwrap().act == Act::Mood {
                napped[sim.monitor()] += Duration::from_secs(10);
            }
        }
        assert!(napped.iter().all(|&time| time >= minutes(5)), "{napped:?}");
        let walks = sim.walks();
        assert!(walks.len() >= 6 && walks.iter().any(|&(_, length)| length > 1000.0), "{walks:?}");
        let walking = sim.plans.iter().filter(|plan| plan.act == Act::Amble).count();
        assert!(walking < sim.plans.len() / 2, "he gets where he's going");
    }

    #[test]
    fn travelling_he_comes_to_the_pointer_on_the_other_monitor_when_claude_needs_you() {
        let mut sim = Sim::new(22, Mood::Waiting).travelling(&two_monitors());
        sim.s.react = true;
        sim.s.at = pos2(1500.0, 600.0);
        sim.find_monitor();
        let pointer = pos2(2300.0, 700.0);
        sim.s.pointer = Some(pointer);
        sim.run(Duration::from_secs(3));
        sim.s.asking = true;
        sim.run(Duration::from_secs(25));
        let plan = *sim.plans.last().unwrap();
        assert_eq!(plan.act, Act::Ask);
        assert_eq!(sim.monitor(), 1);
        assert!(((plan.at + sim.s.feet - pointer).length() - BESIDE.length()).abs() < 3.0, "he's at {:?}", plan.at);
    }

    #[test]
    fn not_travelling_he_keeps_to_his_own_monitor() {
        let mut sim = Sim::new(23, Mood::Asleep);
        sim.s.area = two_monitors()[0].area;
        sim.run(minutes(60));
        // Sim::run checks he's on his own screen every frame.
        assert!(sim.walks().len() >= 6);
    }

    /// The pointer over him, a frame at a time: when, and where.
    fn hover(hover: &mut Hover, start: Instant, path: &[(u64, Pos2)], body: impl Fn(u64) -> Rect, card: Option<Rect>) -> Vec<Look> {
        let at = |ms: u64| start + Duration::from_millis(ms);
        path.iter().map(|&(ms, pointer)| hover.update(at(ms), pointer, body(ms), card, 1.0)).collect()
    }

    fn body() -> Rect {
        Rect::from_min_size(pos2(500.0, 500.0), vec2(80.0, 52.0))
    }

    #[test]
    fn a_pointer_passing_over_him_doesnt_show_the_card() {
        // 1,500 points a second, straight across.
        let path: Vec<(u64, Pos2)> = (0..40).map(|i| (i * 16, pos2(400.0 + 24.0 * i as f32, 520.0))).collect();
        let looks = hover(&mut Hover::default(), Instant::now(), &path, |_| body(), None);
        assert!(looks.iter().all(|&look| look == Look::Away), "{looks:?}");
    }

    #[test]
    fn a_pointer_resting_on_him_shows_the_card_after_a_moment() {
        let rest = pos2(541.0, 521.0);
        let mut path: Vec<(u64, Pos2)> = (0..10).map(|i| (i * 16, pos2(450.0 + 10.0 * i as f32, 520.0))).collect();
        path.extend((10..60).map(|i| (i * 16, rest)));
        let looks = hover(&mut Hover::default(), Instant::now(), &path, |_| body(), None);
        let opened = looks.iter().position(|&look| look == Look::Open).expect("the card never showed");
        let rested = path.iter().position(|&(_, at)| at == rest).unwrap();
        let waited = Duration::from_millis(path[opened].0 - path[rested].0);
        assert!(waited >= REST && waited < REST + Duration::from_millis(150), "{waited:?}");
        assert!(looks[rested..opened].contains(&Look::Coming), "he stops for it");
    }

    #[test]
    fn walking_under_a_still_pointer_isnt_hovering_until_it_moves() {
        let start = Instant::now();
        let still = pos2(560.0, 520.0);
        // He walks right at 40 points a second, under it.
        let path: Vec<(u64, Pos2)> = (0..120).map(|i| (i * 16, still)).collect();
        let walking = |ms: u64| body().translate(vec2(-60.0 + 0.04 * ms as f32, 0.0));
        let mut hovering = Hover::default();
        let looks = hover(&mut hovering, start, &path, walking, None);
        assert!(looks.iter().all(|&look| look == Look::Away), "{looks:?}");
        // A nudge on him says it's meant.
        let nudged: Vec<(u64, Pos2)> =
            (120..170).map(|i| (i * 16, still + vec2(if i < 123 { 2.0 * (i - 119) as f32 } else { 6.0 }, 0.0))).collect();
        let looks = hover(&mut hovering, start, &nudged, |_| body(), None);
        assert_eq!(looks.last(), Some(&Look::Open));
    }

    #[test]
    fn the_card_stays_while_the_pointer_goes_over_to_it_and_goes_once_it_leaves() {
        let start = Instant::now();
        let card = Rect::from_min_size(pos2(600.0, 470.0), vec2(220.0, 120.0));
        let mut hovering = Hover::default();
        hovering.open(start);
        // Across the gap and onto the card.
        let mut path: Vec<(u64, Pos2)> = (1..20).map(|i| (i * 16, pos2(560.0 + 5.0 * i as f32, 520.0))).collect();
        path.extend((20..60).map(|i| (i * 16, pos2(700.0, 520.0))));
        let looks = hover(&mut hovering, start, &path, |_| body(), Some(card));
        assert!(looks.iter().all(|&look| look == Look::Open));
        // Off both, it goes after a moment.
        let away: Vec<(u64, Pos2)> = (60..120).map(|i| (i * 16, pos2(1000.0, 900.0))).collect();
        let looks = hover(&mut hovering, start, &away, |_| body(), Some(card));
        let closed = looks.iter().position(|&look| look == Look::Away).expect("the card never went");
        let waited = Duration::from_millis(away[closed].0 - away[0].0);
        assert!(waited >= LINGER && waited < LINGER + Duration::from_millis(50), "{waited:?}");
    }

    #[test]
    fn his_eyes_turn_to_the_pointer() {
        let eye = pos2(500.0, 500.0);
        assert_eq!(eyes(eye, pos2(1500.0, 520.0), 1.0), [2, 0], "far off to the right");
        assert_eq!(eyes(eye, pos2(400.0, 480.0), 1.0), [-1, 0], "a little way left");
        assert_eq!(eyes(eye, pos2(560.0, 380.0), 1.0), [1, -1], "up and right");
        assert_eq!(eyes(eye, pos2(500.0, 900.0), 1.0), [0, 1], "straight down");
        assert_eq!(eyes(eye, pos2(505.0, 505.0), 1.0), [0, 0], "right on him");
        // Twice the pixels per point: the same pixels are half as far.
        assert_eq!(eyes(eye, pos2(800.0, 500.0), 1.0), [2, 0]);
        assert_eq!(eyes(eye, pos2(800.0, 500.0), 2.0), [1, 0]);
    }
}
