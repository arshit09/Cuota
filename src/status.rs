//! Clawd's status above the meters, drawn like Claude Code's spinner: a star that turns while
//! Claude works, then what Clawd's up to. A new word types itself over the last one, a cursor
//! sweeping across with the letters just behind it still settling.

use crate::mascot::CLAUDE_ORANGE;
use crate::theme::Palette;
use eframe::egui::{Color32, FontId, Painter, Pos2, Rect, Vec2, pos2, vec2};
use std::time::{Duration, Instant};

/// Text size, in points at the panel's normal size.
const SIZE: f32 = 12.5;
/// Claude Code's spinner on Windows: the star grows through these, then shrinks back.
const STARS: [char; 6] = ['·', '✢', '*', '✶', '✻', '✽'];
const STAR_TIME: Duration = Duration::from_millis(120);
/// The star at rest, as Claude Code leaves it once a turn is over.
const RESTING: char = '✻';
/// The cursor moves on a letter this often...
const LETTER_TIME: Duration = Duration::from_millis(40);
/// ...and the letters this close behind it are still settling, flickering through these.
const SETTLING: usize = 2;
const NOISE: [char; 2] = ['.', '_'];
/// Room for the star, and between it and the word, in ems.
const STAR_WIDTH: f32 = 1.0;
const STAR_GAP: f32 = 0.35;
/// The cursor's width, and the room it takes in the line, in ems.
const CURSOR_WIDTH: f32 = 0.32;
const CURSOR_ROOM: f32 = 0.7;

pub struct Status {
    word: String,
    /// The word being typed over, and when the cursor set off.
    typing: Option<(String, Instant)>,
    /// When the star started turning.
    since: Instant,
    /// A rim of this colour around everything, so the line reads over whatever is behind it
    /// when there's no panel: the desktop pet's.
    pub outline: Option<Color32>,
}

impl Status {
    pub fn new() -> Self {
        Self { word: String::new(), typing: None, since: Instant::now(), outline: None }
    }

    /// Shows `word`, typed over the last one when `animate`.
    pub fn set(&mut self, word: &str, animate: bool) {
        if word != self.word {
            let last = std::mem::replace(&mut self.word, word.to_owned());
            self.typing = (animate && !last.is_empty()).then(|| (last, Instant::now()));
        }
    }

    /// How much room the line takes at `scale`: the star, then the word, or the wider of the
    /// two words while one types over the other.
    pub fn size(&self, painter: &Painter, scale: f32) -> Vec2 {
        let font = FontId::proportional(SIZE * scale);
        let width = |text: &str| painter.layout_no_wrap(text.to_owned(), font.clone(), Color32::PLACEHOLDER).size().x;
        let mut word = width(&self.word);
        if let Some((last, _)) = &self.typing {
            word = word.max(width(last)) + CURSOR_ROOM * font.size;
        }
        let height = painter.fonts_mut(|fonts| fonts.row_height(&font));
        vec2((STAR_WIDTH + STAR_GAP) * font.size + word, height)
    }

    /// Paints the line with its top-left at `at`, in `palette`'s colors. While `working`, the
    /// star is orange and turns (if `animate`) and the word is bright; otherwise both are dim
    /// and still. Returns when it next changes.
    pub fn paint(
        &mut self,
        painter: &Painter,
        at: Pos2,
        working: bool,
        animate: bool,
        scale: f32,
        palette: &Palette,
    ) -> Option<Duration> {
        let now = Instant::now();
        let font = FontId::proportional(SIZE * scale);
        let em = font.size;
        let (star_color, color) = if working { (CLAUDE_ORANGE, palette.text) } else { (palette.dim, palette.dim) };
        let turning = working && animate;
        let turned = now.saturating_duration_since(self.since);
        let glyph = if turning { star(turned) } else { RESTING };
        // A pixel's rim in every direction, under the text.
        let outline = self.outline;
        let rim = |text: &str, at: Pos2| {
            let Some(outline) = outline else { return };
            let galley = painter.layout_no_wrap(text.to_owned(), font.clone(), outline);
            let pixel = 1.0 / painter.pixels_per_point();
            for (dx, dy) in [(-1.0, -1.0), (0.0, -1.0), (1.0, -1.0), (-1.0, 0.0), (1.0, 0.0), (-1.0, 1.0), (0.0, 1.0), (1.0, 1.0)] {
                painter.galley(at + vec2(dx, dy) * pixel, galley.clone(), outline);
            }
        };
        // Centred in its room, on the word's baseline.
        let galley = painter.layout_no_wrap(glyph.to_string(), font.clone(), star_color);
        let star_at = pos2(at.x + (STAR_WIDTH * em - galley.size().x) / 2.0, at.y);
        rim(&glyph.to_string(), star_at);
        painter.galley(star_at, galley, star_color);

        let mut x = at.x + (STAR_WIDTH + STAR_GAP) * em;
        let typing = self.typing.as_ref().filter(|_| animate).and_then(|(last, since)| {
            let elapsed = now.saturating_duration_since(*since);
            typed(last, &self.word, elapsed).map(|text| (text, elapsed))
        });
        if typing.is_none() {
            self.typing = None;
        }
        let text = |text: &str, x: &mut f32| {
            let galley = painter.layout_no_wrap(text.to_owned(), font.clone(), color);
            *x += galley.size().x;
            let height = galley.size().y;
            let left = pos2(*x - galley.size().x, at.y);
            rim(text, left);
            painter.galley(left, galley, color);
            height
        };
        let mut next = None;
        match &typing {
            None => {
                text(&self.word, &mut x);
            }
            Some(((before, after), elapsed)) => {
                let height = text(before, &mut x);
                let cursor = Rect::from_min_size(pos2(x + 0.06 * em, at.y + 0.12 * height), vec2(CURSOR_WIDTH * em, 0.8 * height));
                if let Some(outline) = outline {
                    painter.rect_filled(cursor.expand(1.0 / painter.pixels_per_point()), 0.0, outline);
                }
                painter.rect_filled(cursor, 0.0, color);
                x += CURSOR_ROOM * em;
                text(after, &mut x);
                next = Some(until_next(*elapsed, LETTER_TIME));
            }
        }
        if turning {
            let turn = until_next(turned, STAR_TIME);
            next = Some(next.map_or(turn, |next: Duration| next.min(turn)));
        }
        next
    }
}

/// The star showing `turned` into turning: up through [`STARS`] and back down.
fn star(turned: Duration) -> char {
    let step = (turned.as_millis() / STAR_TIME.as_millis()) as usize % (2 * STARS.len());
    STARS[if step < STARS.len() { step } else { 2 * STARS.len() - 1 - step }]
}

/// What shows `elapsed` into typing `to` over `from`: the letters before the cursor and after
/// it, or `None` once the cursor has passed both words.
fn typed(from: &str, to: &str, elapsed: Duration) -> Option<(String, String)> {
    let (from, to): (Vec<char>, Vec<char>) = (from.chars().collect(), to.chars().collect());
    let cursor = (elapsed.as_millis() / LETTER_TIME.as_millis()) as usize;
    if cursor > from.len().max(to.len()) {
        return None;
    }
    let before = to.iter().take(cursor).enumerate().map(|(i, &c)| {
        if c == ' ' || cursor - i > SETTLING {
            return c;
        }
        // Each settling letter picks noise, or itself, afresh with each step of the cursor.
        let mixed = (i as u32).wrapping_mul(0x9E37_79B9) ^ (cursor as u32).wrapping_mul(0x85EB_CA6B);
        let mixed = mixed ^ (mixed >> 15);
        if mixed.is_multiple_of(3) { c } else { NOISE[(mixed >> 4) as usize % NOISE.len()] }
    });
    // The cursor stands on the old letter it's replacing.
    Some((before.collect(), from.iter().skip(cursor + 1).collect()))
}

/// How long until the next step of something that steps every `step` and has been going for
/// `elapsed`.
fn until_next(elapsed: Duration, step: Duration) -> Duration {
    let step = step.as_nanos();
    Duration::from_nanos((step - elapsed.as_nanos() % step) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    #[test]
    fn the_star_grows_and_shrinks_back_like_claude_codes() {
        let turn: String = (0..13).map(|i| star(STAR_TIME * i)).collect();
        assert_eq!(turn, "·✢*✶✻✽✽✻✶*✢··");
        assert_eq!(star(STAR_TIME * 3 + ms(119)), '✶');
    }

    #[test]
    fn a_new_word_types_over_the_last_one() {
        assert_eq!(typed("Mustering…", "Forging…", ms(0)), Some((String::new(), "ustering…".into())));
        // Past the settling letters, the new word shows as it is; the old one waits after the cursor.
        let (before, after) = typed("Mustering…", "Forging…", LETTER_TIME * 5 + ms(1)).unwrap();
        assert!(before.starts_with("For") && before.chars().count() == 5, "{before}");
        assert_eq!(after, "ing…");
        let (before, after) = typed("Mustering…", "Forging…", LETTER_TIME * 10).unwrap();
        assert!(before.starts_with("Forgin") && before.chars().count() == 8, "{before}");
        assert_eq!(after, "");
        assert_eq!(typed("Mustering…", "Forging…", LETTER_TIME * 11), None);
    }

    #[test]
    fn settling_letters_flicker_through_noise_and_land_on_themselves() {
        let mut noisy = false;
        for cursor in 1..=17 {
            let (before, _) = typed("", "Discombobulating…", LETTER_TIME * cursor).unwrap();
            for (i, (shown, real)) in before.chars().zip("Discombobulating…".chars()).enumerate() {
                if (cursor as usize) - i > SETTLING {
                    assert_eq!(shown, real, "letter {i} at step {cursor}");
                } else {
                    assert!(shown == real || NOISE.contains(&shown), "{shown}");
                    noisy |= shown != real;
                }
            }
        }
        assert!(noisy);
        // Spaces stay put, so the words keep their shape.
        assert_eq!(typed("", "All done!", LETTER_TIME * 4).unwrap().0.chars().nth(3), Some(' '));
    }

    #[test]
    fn steps_come_due_on_time() {
        assert_eq!(until_next(ms(0), LETTER_TIME), LETTER_TIME);
        assert_eq!(until_next(ms(130), STAR_TIME), ms(110));
    }
}
