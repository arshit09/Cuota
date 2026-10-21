//! Moving the panel out of the pointer's way. A pointer that slows down as it comes toward the
//! panel, or on it, is about to click something there, so the panel slides aside, clear of the
//! spot it sat on, and back once the pointer has left that spot. A pointer passing by quickly,
//! or idling beside the panel, leaves it be, and with Ctrl held the pointer can reach it.
//!
//! A thread of its own watches the pointer ([`Planner`] decides), so the panel only draws a
//! frame while it slides ([`Dodge`]).

use crate::win;
use eframe::egui::{self, Pos2, Rect, Vec2, pos2};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::thread::Thread;
use std::time::{Duration, Instant};

/// Slower than this, in points a second, a pointer is homing in on something rather than
/// passing by.
const SLOW: f32 = 300.0;
/// How far back the pointer's speed is measured.
const SPEED_SPAN: Duration = Duration::from_millis(60);
/// A pointer this close to the panel, in points, is on it.
const ON: f32 = 6.0;
/// A pointer within this many points of the panel heads for it once it has come `CLOSER`
/// points nearer within `TREND_SPAN`.
const NEAR: f32 = 40.0;
const CLOSER: f32 = 8.0;
const TREND_SPAN: Duration = Duration::from_millis(300);
/// How long the pointer homes in before the panel moves, so a pointer only slowing for a
/// moment as it passes leaves it be.
const DWELL: Duration = Duration::from_millis(60);
/// The panel heads back once the pointer has been more than this many points off its spot
/// for `RETURN_DELAY`.
const LEAVE: f32 = 16.0;
const RETURN_DELAY: Duration = Duration::from_millis(500);
/// Between the panel's spot and where it waits aside, in points. More than `LEAVE`, so a
/// pointer on the panel aside is always off its spot, and the panel goes back.
const GAP: f32 = 24.0;
/// After heading back, the panel's spot is kept until its window gets there, or this long.
const LANDING: Duration = Duration::from_secs(1);
/// How often the pointer is looked at near the panel, and farther off.
const NEAR_TICK: Duration = Duration::from_millis(16);
const FAR_TICK: Duration = Duration::from_millis(50);
/// Sliding aside, quickly to uncover the pointer soon, and back, more gently.
const OUT_TIME: Duration = Duration::from_millis(200);
const BACK_TIME: Duration = Duration::from_millis(300);

/// What the watcher saw, in physical pixels.
#[derive(Clone, Copy, Debug)]
pub struct Sample {
    pub at: Instant,
    pub pointer: Pos2,
    /// Either Ctrl key is down.
    pub ctrl: bool,
    /// A mouse button is down.
    pub pressed: bool,
    /// The panel's window, where it is now.
    pub panel: Rect,
    /// The part of the panel's monitor that the taskbar leaves free.
    pub work: Rect,
    /// Pixels per point.
    pub scale: f32,
    /// The panel shows at its point nearest the pointer, rather than a window over it.
    pub visible: bool,
}

/// Where the panel sits when it isn't aside (its window's top-left), and how far aside it is:
/// zero on its way back. Physical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aside {
    pub home: Pos2,
    pub offset: Vec2,
}

/// Which way the panel went.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Side {
    Up,
    Down,
    Left,
    Right,
}

impl Side {
    /// How far a panel of `size` moves this way to clear its spot by `gap`.
    fn offset(self, size: Vec2, gap: f32) -> Vec2 {
        match self {
            Self::Up => Vec2::new(0.0, -(size.y + gap)),
            Self::Down => Vec2::new(0.0, size.y + gap),
            Self::Left => Vec2::new(-(size.x + gap), 0.0),
            Self::Right => Vec2::new(size.x + gap, 0.0),
        }
    }
}

/// The way to slide the panel off its `spot`: the shorter way first, and away from the pointer
/// first, so the pointer is uncovered soonest. `None` when no way keeps it on `work`.
fn way_out(spot: Rect, pointer: Pos2, work: Rect, gap: f32) -> Option<Side> {
    let vertical = if pointer.y >= spot.center().y { [Side::Up, Side::Down] } else { [Side::Down, Side::Up] };
    let horizontal = if pointer.x >= spot.center().x { [Side::Left, Side::Right] } else { [Side::Right, Side::Left] };
    let (first, then) = if spot.height() <= spot.width() { (vertical, horizontal) } else { (horizontal, vertical) };
    first.into_iter().chain(then).find(|side| work.contains_rect(spot.translate(side.offset(spot.size(), gap))))
}

/// Decides, from what the watcher sees, when the panel moves aside and when it comes back.
#[derive(Default)]
pub struct Planner {
    /// Where the pointer was lately, and when.
    trail: VecDeque<(Instant, Pos2)>,
    /// Since when the pointer has been homing in on the panel.
    homing_since: Option<Instant>,
    /// The panel's top-left on its spot, while it's aside or on its way back.
    home: Option<Pos2>,
    /// Which way it went, while it's aside.
    side: Option<Side>,
    /// Since when the pointer has been off the panel's spot, while it's aside.
    left_since: Option<Instant>,
    /// When the panel started back.
    back_since: Option<Instant>,
    /// The pointer reached the panel (with Ctrl held, or by dragging it), so the panel stays
    /// put until the pointer leaves it.
    reached: bool,
    /// Where the panel was when a mouse button went down on it, while that's held.
    press: Option<Rect>,
    pressed: bool,
}

impl Planner {
    /// Where the panel should be, given what the watcher saw: `None` while it stays on its spot.
    pub fn update(&mut self, sample: &Sample) -> Option<Aside> {
        let now = sample.at;
        self.trail.push_back((now, sample.pointer));
        // The last TREND_SPAN, and the one before it to measure from.
        while self.trail.get(1).is_some_and(|&(at, _)| now - at >= TREND_SPAN) {
            self.trail.pop_front();
        }
        let spot = self.home.map_or(sample.panel, |home| Rect::from_min_size(home, sample.panel.size()));
        let scale = sample.scale;

        // A button pressed on the panel keeps it still while held, and if that dragged it,
        // until the pointer leaves it.
        let on_panel = sample.panel.contains(sample.pointer);
        if sample.pressed && !self.pressed && on_panel {
            self.press = Some(sample.panel);
            // It was pressed as the panel set off, before it moved: the click got there first.
            if self.side.is_some() && self.home == Some(sample.panel.min) {
                (self.home, self.side) = (None, None);
            }
        }
        if !sample.pressed
            && let Some(pressed) = self.press.take()
            && pressed.min != sample.panel.min
        {
            self.reached = true;
        }
        self.pressed = sample.pressed;
        if sample.ctrl && on_panel && self.side.is_none() {
            self.reached = true;
        }
        if self.reached && !sample.panel.expand(ON * scale).contains(sample.pointer) {
            self.reached = false;
        }

        match self.side {
            Some(_) => {
                if spot.expand(LEAVE * scale).contains(sample.pointer) {
                    self.left_since = None;
                } else if now - *self.left_since.get_or_insert(now) >= RETURN_DELAY {
                    (self.side, self.left_since, self.back_since) = (None, None, Some(now));
                }
            }
            None => {
                // Back on its spot once the window gets there, or should have by now.
                if let Some(home) = self.home
                    && ((sample.panel.min - home).length() <= 1.0 || self.back_since.is_none_or(|at| now - at >= LANDING))
                {
                    self.home = None;
                }
                let still = self.reached || self.press.is_some() || sample.ctrl || !sample.visible;
                if !still && self.homing_in(sample, spot) {
                    let since = *self.homing_since.get_or_insert(now);
                    if now - since >= DWELL
                        && let Some(side) = way_out(spot, sample.pointer, sample.work, GAP * scale)
                    {
                        (self.home, self.side, self.homing_since) = (Some(spot.min), Some(side), None);
                    }
                } else {
                    self.homing_since = None;
                }
            }
        }
        let offset = |side: Side| side.offset(sample.panel.size(), GAP * scale);
        self.home.map(|home| Aside { home, offset: self.side.map_or(Vec2::ZERO, offset) })
    }

    /// The pointer is slow, and on the panel's `spot`, or near it and coming closer.
    fn homing_in(&self, sample: &Sample, spot: Rect) -> bool {
        let points_off = |at: Pos2| spot.distance_to_pos(at) / sample.scale;
        let off = points_off(sample.pointer);
        if off > NEAR || self.speed(sample.at) / sample.scale > SLOW {
            return false;
        }
        let farthest = self.trail.iter().map(|&(_, at)| points_off(at)).fold(0.0, f32::max);
        off <= ON || farthest - off >= CLOSER
    }

    /// How fast the pointer went lately, in pixels a second: since the newest sample at least
    /// `SPEED_SPAN` old, or the oldest.
    fn speed(&self, now: Instant) -> f32 {
        let Some(&(_, to)) = self.trail.back() else { return 0.0 };
        let from = self.trail.iter().rev().find(|&&(at, _)| now - at >= SPEED_SPAN).or(self.trail.front());
        let Some(&(then, from)) = from else { return 0.0 };
        let seconds = (now - then).as_secs_f32();
        if seconds > 0.0 { (to - from).length() / seconds } else { 0.0 }
    }

    /// Whether the panel is aside or on its way back.
    fn away(&self) -> bool {
        self.home.is_some()
    }
}

/// Between the panel and the watcher thread.
#[derive(Default)]
struct Shared {
    /// The panel wants moving out of the pointer's way now.
    armed: bool,
    /// The watcher's latest plan for it.
    aside: Option<Aside>,
    /// The panel's window is gone (a copy on another monitor closed), so the watcher ends.
    gone: bool,
}

fn to_rect(rect: windows_sys::Win32::Foundation::RECT) -> Rect {
    Rect::from_min_max(pos2(rect.left as f32, rect.top as f32), pos2(rect.right as f32, rect.bottom as f32))
}

/// What the pointer and the panel's window are doing now.
fn sample(window: isize) -> Sample {
    let pointer = win::cursor();
    let pointer = pos2(pointer.x as f32, pointer.y as f32);
    let panel = to_rect(win::window_rect(window));
    let center = panel.center();
    let work = win::work_area(center.x as i32, center.y as i32).map_or(Rect::EVERYTHING, to_rect);
    let scale = win::window_scale(window);
    // Only asked near the panel, the only place it counts: at its point nearest the pointer,
    // inside its rounded corners.
    let visible = panel.expand(NEAR * scale).contains(pointer) && {
        let inset = win::corner_radius().min(panel.width().min(panel.height()) / 2.0 - 1.0).max(0.0);
        let probe = panel.shrink(inset).clamp(pointer);
        win::shows_at(window, probe.x as i32, probe.y as i32)
    };
    Sample {
        at: Instant::now(),
        pointer,
        ctrl: win::ctrl_down(),
        pressed: win::mouse_down(),
        panel,
        work,
        scale,
        visible,
    }
}

/// Watches the pointer while the panel is armed, and wakes the panel whenever the plan for it
/// changes. Sleeps while it isn't armed.
fn watch(window: isize, shared: Arc<Mutex<Shared>>, ctx: egui::Context) {
    let mut planner = Planner::default();
    loop {
        let (armed, gone) = {
            let shared = shared.lock().unwrap();
            (shared.armed, shared.gone)
        };
        if gone {
            return;
        }
        if !armed {
            planner = Planner::default();
            std::thread::park();
            continue;
        }
        let sample = sample(window);
        let aside = planner.update(&sample);
        {
            let mut shared = shared.lock().unwrap();
            if shared.armed && shared.aside != aside {
                shared.aside = aside;
                ctx.request_repaint();
            }
        }
        let near = planner.away() || sample.panel.expand(2.0 * NEAR * sample.scale).contains(sample.pointer);
        std::thread::sleep(if near { NEAR_TICK } else { FAR_TICK });
    }
}

/// The panel's slide aside and back, a frame at a time. Points.
struct Glide {
    /// The panel's top-left on its spot, while it's aside, on its way, or not yet seen back.
    home: Option<Pos2>,
    offset: Vec2,
    from: Vec2,
    to: Vec2,
    start: Instant,
    /// When it got back to its spot.
    back_at: Option<Instant>,
}

impl Glide {
    /// On its spot, going nowhere.
    fn new() -> Self {
        Self { home: None, offset: Vec2::ZERO, from: Vec2::ZERO, to: Vec2::ZERO, start: Instant::now(), back_at: None }
    }

    /// Heads for `to` from wherever it is, unless it's already heading there.
    fn aim(&mut self, to: Vec2, now: Instant) {
        if to != self.to {
            (self.from, self.to, self.start) = (self.offset, to, now);
        }
    }

    /// Straight back to its spot.
    fn stop(&mut self) {
        (self.offset, self.from, self.to) = (Vec2::ZERO, Vec2::ZERO, Vec2::ZERO);
    }

    /// Moves along. Returns whether it has further to go.
    fn step(&mut self, now: Instant) -> bool {
        if self.offset == self.to {
            return false;
        }
        let back = self.to == Vec2::ZERO;
        let time = if back { BACK_TIME } else { OUT_TIME };
        let t = (now - self.start).as_secs_f32() / time.as_secs_f32();
        if t >= 1.0 {
            self.offset = self.to;
            return false;
        }
        // Out fast then easing off, to uncover the pointer soon; back easing in and out.
        let eased = if back {
            if t < 0.5 { 4.0 * t * t * t } else { 1.0 - (2.0 - 2.0 * t).powi(3) / 2.0 }
        } else {
            1.0 - (1.0 - t).powi(3)
        };
        self.offset = self.from + (self.to - self.from) * eased;
        true
    }

    fn still(&self) -> bool {
        self.offset == Vec2::ZERO && self.to == Vec2::ZERO
    }
}

/// Moves the panel out of the pointer's way, on the watcher thread's plan.
pub struct Dodge {
    shared: Arc<Mutex<Shared>>,
    /// The watcher, once it has been needed.
    watcher: Option<Thread>,
    glide: Glide,
    /// Sliding back because it's no longer wanted, so it ignores the watcher until it's back.
    recalled: bool,
    /// The window's last position sent, in pixels.
    sent: Option<Pos2>,
}

impl Dodge {
    pub fn new() -> Self {
        Self { shared: Arc::default(), watcher: None, glide: Glide::new(), recalled: false, sent: None }
    }

    /// Where the panel sits when it isn't moved aside, while a move aside is under way.
    pub fn home(&self) -> Option<Pos2> {
        self.glide.home
    }

    /// The panel is on its spot, with no move aside under way, so the pointer can use it.
    pub fn idle(&self) -> bool {
        self.glide.home.is_none()
    }

    /// The panel is aside or sliding, so its window is where the slide puts it.
    pub fn moving(&self) -> bool {
        !self.glide.still()
    }

    /// Starts or stops the watcher.
    fn arm(&mut self, ctx: &egui::Context, window: isize, armed: bool) {
        let mut shared = self.shared.lock().unwrap();
        if shared.armed == armed {
            return;
        }
        shared.armed = armed;
        if !armed {
            shared.aside = None;
            return;
        }
        drop(shared);
        match &self.watcher {
            Some(watcher) => watcher.unpark(),
            None => {
                let (shared, ctx) = (self.shared.clone(), ctx.clone());
                let watcher = std::thread::Builder::new().name("dodge".into()).spawn(move || watch(window, shared, ctx));
                self.watcher = watcher.ok().map(|watcher| watcher.thread().clone());
            }
        }
    }

    /// Runs every frame. `wanted`: moving aside is on, and the panel shows where it can. When
    /// it isn't, the panel slides back, or with `jump` (it's hidden, expanding or being placed)
    /// goes straight back. `seen` is where the panel's spot is by its window's latest position,
    /// in points. Returns where to put the window, in points, when it moves.
    pub fn update(
        &mut self,
        ctx: &egui::Context,
        window: isize,
        wanted: bool,
        jump: bool,
        seen: Option<Pos2>,
    ) -> Option<Pos2> {
        let now = Instant::now();
        self.arm(ctx, window, wanted && !self.recalled);
        let plan = self.shared.lock().unwrap().aside;
        let ppp = ctx.pixels_per_point();
        let glide = &mut self.glide;
        match plan {
            Some(aside) => {
                glide.home = Some(aside.home / ppp);
                glide.aim(aside.offset / ppp, now);
            }
            None if !wanted && jump => glide.stop(),
            None => {
                self.recalled |= !wanted && !glide.still();
                glide.aim(Vec2::ZERO, now);
            }
        }
        if glide.step(now) {
            ctx.request_repaint();
        }
        let home = glide.home?;
        if glide.still() {
            self.recalled = false;
            // Done once the window is seen back on its spot, so until then the panel goes on
            // placing itself from there.
            let back_at = *glide.back_at.get_or_insert(now);
            let there = seen.is_some_and(|seen| (seen - home).length() * ppp <= 1.0);
            if plan.is_none() && (there || now - back_at >= LANDING) {
                (glide.home, self.sent) = (None, None);
                return None;
            }
        } else {
            glide.back_at = None;
        }
        let at = home + glide.offset;
        let pixels = (at * ppp).round();
        (self.sent != Some(pixels)).then(|| {
            self.sent = Some(pixels);
            at
        })
    }
}

impl Default for Dodge {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Dodge {
    fn drop(&mut self) {
        self.shared.lock().unwrap().gone = true;
        if let Some(watcher) = &self.watcher {
            watcher.unpark();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::vec2;

    const TICK: Duration = Duration::from_millis(16);

    /// The watcher's view, a tick at a time, with the window going wherever the plan puts it.
    struct Sim {
        planner: Planner,
        now: Instant,
        pointer: Pos2,
        panel: Rect,
        work: Rect,
        ctrl: bool,
        pressed: bool,
        visible: bool,
        plan: Option<Aside>,
    }

    impl Sim {
        /// A 300×100 panel in the middle of a 1920×1040 work area.
        fn new() -> Self {
            Self::at(Rect::from_min_size(pos2(800.0, 500.0), vec2(300.0, 100.0)))
        }

        fn at(panel: Rect) -> Self {
            Self {
                planner: Planner::default(),
                now: Instant::now(),
                pointer: pos2(100.0, 100.0),
                panel,
                work: Rect::from_min_size(Pos2::ZERO, vec2(1920.0, 1040.0)),
                ctrl: false,
                pressed: false,
                visible: true,
                plan: None,
            }
        }

        fn tick(&mut self) -> Option<Aside> {
            self.now += TICK;
            let sample = Sample {
                at: self.now,
                pointer: self.pointer,
                ctrl: self.ctrl,
                pressed: self.pressed,
                panel: self.panel,
                work: self.work,
                scale: 1.0,
                visible: self.visible,
            };
            self.plan = self.planner.update(&sample);
            if let Some(aside) = self.plan {
                self.panel = Rect::from_min_size(aside.home + aside.offset, self.panel.size());
            }
            self.plan
        }

        /// Moves the pointer steadily to `to` over `time`. Returns whether the panel was aside
        /// at any point on the way.
        fn move_to(&mut self, to: Pos2, time: Duration) -> bool {
            let (from, ticks) = (self.pointer, (time.as_millis() / TICK.as_millis()).max(1) as usize);
            let mut aside = false;
            for i in 1..=ticks {
                self.pointer = from + (to - from) * (i as f32 / ticks as f32);
                aside |= self.tick().is_some_and(|plan| plan.offset != Vec2::ZERO);
            }
            aside
        }

        fn rest(&mut self, time: Duration) -> bool {
            self.move_to(self.pointer, time)
        }

        fn aside(&self) -> bool {
            self.plan.is_some_and(|plan| plan.offset != Vec2::ZERO)
        }
    }

    fn ms(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    #[test]
    fn a_pointer_passing_quickly_over_the_panel_leaves_it_be() {
        let mut sim = Sim::new();
        sim.pointer = pos2(600.0, 550.0);
        assert!(!sim.move_to(pos2(1300.0, 560.0), ms(300)), "across it at over 2000 px/s");
        assert!(!sim.move_to(pos2(950.0, 300.0), ms(200)));
        assert_eq!(sim.plan, None);
    }

    #[test]
    fn a_pointer_slowing_as_it_comes_toward_the_panel_moves_it_aside_before_it_gets_there() {
        let mut sim = Sim::new();
        sim.pointer = pos2(950.0, 900.0);
        // Quickly up toward it, then slowly the last stretch, to a point just inside its bottom edge.
        sim.move_to(pos2(950.0, 640.0), ms(150));
        assert!(!sim.aside());
        let mut moved_at = None;
        for step in 1..=50 {
            sim.pointer = pos2(950.0, 640.0 - step as f32 * 1.2);
            if sim.tick().is_some() && moved_at.is_none() {
                moved_at = Some(sim.pointer);
            }
        }
        let moved_at = moved_at.expect("it moved aside");
        assert!(moved_at.y > 600.0, "it moved while the pointer was still below it, at {moved_at:?}");
        // Up, away from the pointer below its middle, the shorter way, and clear of its spot.
        let plan = sim.plan.unwrap();
        assert_eq!(plan.home, pos2(800.0, 500.0));
        assert_eq!(plan.offset, vec2(0.0, -(100.0 + GAP)));
        assert!(!sim.panel.expand(LEAVE).contains(sim.pointer));
    }

    #[test]
    fn the_panel_waits_aside_while_the_pointer_stays_on_its_spot_and_comes_back_once_it_leaves() {
        let mut sim = Sim::new();
        sim.pointer = pos2(1200.0, 450.0);
        sim.move_to(pos2(950.0, 530.0), ms(1000));
        assert!(sim.aside(), "slowly onto it from above");
        assert_eq!(sim.plan.unwrap().offset, vec2(0.0, 100.0 + GAP), "so down");
        // Wandering around the spot, it stays aside.
        sim.move_to(pos2(820.0, 590.0), ms(1500));
        assert!(sim.aside());
        // Off the spot, it heads back after a moment, and once there the plan is done.
        sim.move_to(pos2(500.0, 200.0), ms(100));
        sim.rest(RETURN_DELAY - ms(100));
        assert!(sim.aside(), "not yet");
        sim.rest(ms(150));
        assert_eq!(sim.plan, None);
        assert_eq!(sim.panel.min, pos2(800.0, 500.0));
        // Back on its spot, it goes again, and off it only briefly, it waits aside.
        sim.move_to(pos2(950.0, 550.0), ms(800));
        sim.rest(ms(200));
        assert!(sim.aside());
        sim.move_to(pos2(950.0, 300.0), ms(100));
        sim.rest(ms(200));
        sim.move_to(pos2(950.0, 550.0), ms(100));
        sim.rest(ms(1000));
        assert!(sim.aside());
    }

    #[test]
    fn a_pointer_idling_beside_the_panel_leaves_it_be_but_one_resting_on_it_does_not() {
        let mut sim = Sim::new();
        sim.pointer = pos2(800.0, 620.0);
        assert!(!sim.move_to(pos2(1100.0, 620.0), ms(2000)), "slowly along it, 20 px below");
        assert!(!sim.rest(ms(1000)));
        let mut sim = Sim::new();
        sim.pointer = pos2(900.0, 550.0);
        assert!(sim.rest(ms(200)));
    }

    #[test]
    fn with_ctrl_held_the_pointer_reaches_the_panel_and_keeps_it_until_leaving_it() {
        let mut sim = Sim::new();
        sim.pointer = pos2(950.0, 800.0);
        sim.ctrl = true;
        assert!(!sim.move_to(pos2(950.0, 560.0), ms(1000)));
        sim.ctrl = false;
        assert!(!sim.rest(ms(1000)), "still on it after letting go of Ctrl");
        assert!(!sim.move_to(pos2(1095.0, 590.0), ms(500)));
        // Off it, and back slowly: now it moves.
        sim.move_to(pos2(950.0, 800.0), ms(100));
        assert!(sim.move_to(pos2(950.0, 560.0), ms(1000)));
    }

    #[test]
    fn a_button_pressed_on_the_panel_holds_it_and_one_that_dragged_it_until_the_pointer_leaves() {
        // A click: held still, then out of the way.
        let mut sim = Sim::new();
        sim.pointer = pos2(900.0, 550.0);
        sim.pressed = true;
        assert!(!sim.rest(ms(500)));
        sim.pressed = false;
        assert!(sim.rest(ms(100)));

        // A drag: it stays where it was dropped, under the pointer, until the pointer leaves.
        let mut sim = Sim::new();
        sim.pointer = pos2(900.0, 550.0);
        sim.pressed = true;
        sim.tick();
        for _ in 0..20 {
            sim.pointer += vec2(5.0, 2.0);
            sim.panel = sim.panel.translate(vec2(5.0, 2.0));
            assert!(sim.tick().is_none());
        }
        sim.pressed = false;
        assert!(!sim.rest(ms(1000)));
        sim.move_to(sim.pointer + vec2(0.0, 300.0), ms(100));
        assert!(sim.move_to(sim.panel.center(), ms(1500)), "back slowly, it moves aside");
        assert_eq!(sim.plan.unwrap().home, pos2(900.0, 540.0), "from where it was dropped");
    }

    #[test]
    fn a_panel_covered_by_another_window_stays_put() {
        let mut sim = Sim::new();
        sim.visible = false;
        sim.pointer = pos2(900.0, 550.0);
        assert!(!sim.rest(ms(500)));
    }

    #[test]
    fn by_the_screens_edges_the_panel_goes_whichever_way_keeps_it_on_screen() {
        let way = |panel: Rect, pointer: Pos2| {
            let mut sim = Sim::at(panel);
            sim.pointer = pointer;
            sim.rest(ms(200));
            sim.plan.map(|plan| plan.offset)
        };
        let corner = Rect::from_min_size(Pos2::ZERO, vec2(300.0, 100.0));
        // Pointer in its lower half, so up first, but there's no room up there.
        assert_eq!(way(corner, pos2(150.0, 80.0)), Some(vec2(0.0, 100.0 + GAP)));
        let bottom = Rect::from_min_size(pos2(0.0, 940.0), vec2(300.0, 100.0));
        assert_eq!(way(bottom, pos2(150.0, 950.0)), Some(vec2(0.0, -(100.0 + GAP))));
        // A tall panel goes sideways, the shorter way, away from the pointer.
        let tall = Rect::from_min_size(pos2(800.0, 300.0), vec2(150.0, 400.0));
        assert_eq!(way(tall, pos2(820.0, 500.0)), Some(vec2(150.0 + GAP, 0.0)));
        // As big as the screen, it has nowhere to go.
        let mut sim = Sim::at(Rect::from_min_size(Pos2::ZERO, vec2(1920.0, 1040.0)));
        sim.pointer = pos2(900.0, 500.0);
        assert!(!sim.rest(ms(500)));
    }

    #[test]
    fn the_slide_eases_out_to_its_place_and_turns_back_from_wherever_it_got_to() {
        let start = Instant::now();
        let mut glide = Glide::new();
        glide.aim(vec2(0.0, -120.0), start);
        let mut last = 0.0;
        for i in 1..=20 {
            let moving = glide.step(start + OUT_TIME * i / 20);
            assert!(glide.offset.y < last, "it keeps going");
            assert_eq!(moving, i < 20);
            last = glide.offset.y;
        }
        assert_eq!(glide.offset, vec2(0.0, -120.0));
        // Fast at first: past halfway in a quarter of the time.
        let mut glide = Glide::new();
        glide.aim(vec2(0.0, -120.0), start);
        let midway = start + OUT_TIME / 4;
        glide.step(midway);
        assert!(glide.offset.y < -60.0, "{:?}", glide.offset);
        // Called back midway, it heads home from there, gently at first.
        glide.aim(Vec2::ZERO, midway);
        glide.step(midway + BACK_TIME / 10);
        assert!(glide.offset.y < -60.0 && glide.offset.y > -75.0, "{:?}", glide.offset);
        glide.step(midway + BACK_TIME);
        assert!(glide.still());
    }
}
