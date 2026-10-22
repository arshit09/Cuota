//! Clawd's Mascot Studio clips, as Cuota's panel plays them, decoded at half size.

use eframe::egui::{Color32, ColorImage, Context, TextureHandle, TextureOptions};
use gif::{ColorOutput, DecodeOptions, DisposalMethod};
use std::time::{Duration, Instant};

pub const GREETING: &[u8] = include_bytes!("../../assets/mascot/greeting.gif");
pub const WORKING: &[u8] = include_bytes!("../../assets/mascot/typing.gif");
pub const WAITING: &[u8] = include_bytes!("../../assets/mascot/waiting-for-you.gif");
pub const DONE: &[u8] = include_bytes!("../../assets/mascot/done.gif");
pub const ERROR: &[u8] = include_bytes!("../../assets/mascot/error.gif");

/// Plays one clip at a time on a loop, into a single texture.
pub struct Player {
    texture: TextureHandle,
    gif: &'static [u8],
    frames: Vec<(ColorImage, Duration)>,
    started: Instant,
    shown: Option<usize>,
}

impl Player {
    pub fn new(ctx: &Context, gif: &'static [u8]) -> Self {
        let frames = decode(gif);
        let texture = ctx.load_texture("clawd", frames[0].0.clone(), TextureOptions::LINEAR);
        Self { texture, gif, frames, started: Instant::now(), shown: Some(0) }
    }

    /// Switches to `gif` from its start, unless it's playing already.
    pub fn play(&mut self, gif: &'static [u8]) {
        if !std::ptr::eq(gif, self.gif) {
            *self = Self { gif, frames: decode(gif), started: Instant::now(), shown: None, texture: self.texture.clone() };
        }
    }

    /// The texture showing the frame for now, its size in GIF pixels, and when the next is due.
    pub fn frame(&mut self) -> (&TextureHandle, [usize; 2], Duration) {
        let length = self.frames.last().unwrap().1;
        let position = Duration::from_secs_f64(self.started.elapsed().as_secs_f64() % length.as_secs_f64());
        let index = self.frames.partition_point(|f| f.1 <= position).min(self.frames.len() - 1);
        if self.shown != Some(index) {
            self.texture.set(self.frames[index].0.clone(), TextureOptions::LINEAR);
            self.shown = Some(index);
        }
        let (image, end) = &self.frames[index];
        (&self.texture, image.size, end.saturating_sub(position))
    }
}

/// Each frame composited at full size and shrunk by half, with when it ends from the start.
/// A clip that won't decode is one empty frame.
fn decode(gif: &[u8]) -> Vec<(ColorImage, Duration)> {
    let mut options = DecodeOptions::new();
    options.set_color_output(ColorOutput::RGBA);
    let mut frames = Vec::new();
    if let Ok(mut decoder) = options.read_info(gif) {
        let (width, height) = (usize::from(decoder.width()), usize::from(decoder.height()));
        let mut canvas = vec![0u8; width * height * 4];
        let mut end = Duration::ZERO;
        while let Ok(Some(frame)) = decoder.read_next_frame() {
            let (left, top, stride) = (usize::from(frame.left), usize::from(frame.top), usize::from(frame.width));
            let w = stride.min(width.saturating_sub(left));
            let h = usize::from(frame.height).min(height.saturating_sub(top));
            for (y, row) in frame.buffer.chunks_exact(stride * 4).enumerate().take(h) {
                let dest = &mut canvas[((top + y) * width + left) * 4..][..w * 4];
                for (d, s) in dest.chunks_exact_mut(4).zip(row.chunks_exact(4)) {
                    if s[3] != 0 {
                        d.copy_from_slice(s);
                    }
                }
            }
            // Players treat no delay as a tenth of a second.
            let delay = if frame.delay == 0 { 10 } else { frame.delay };
            end += Duration::from_millis(u64::from(delay) * 10);
            frames.push((half(&canvas, width, height), end));
            if frame.dispose == DisposalMethod::Background {
                for row in canvas.chunks_exact_mut(width * 4).skip(top).take(h) {
                    row[left * 4..(left + w) * 4].fill(0);
                }
            }
        }
    }
    if frames.is_empty() {
        frames.push((ColorImage::filled([2, 2], Color32::TRANSPARENT), Duration::from_secs(1)));
    }
    frames
}

/// `canvas` at half size, each pixel the average of four, weighted by how opaque they are.
fn half(canvas: &[u8], width: usize, height: usize) -> ColorImage {
    let size = [width / 2, height / 2];
    let mut pixels = Vec::with_capacity(size[0] * size[1]);
    for y in 0..size[1] {
        for x in 0..size[0] {
            let mut sum = [0u32; 4];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let p = &canvas[((2 * y + dy) * width + 2 * x + dx) * 4..][..4];
                let alpha = u32::from(p[3]);
                for c in 0..3 {
                    sum[c] += u32::from(p[c]) * alpha;
                }
                sum[3] += alpha;
            }
            let color = |c: usize| if sum[3] == 0 { 0 } else { (sum[c] / sum[3]) as u8 };
            pixels.push(Color32::from_rgba_unmultiplied(color(0), color(1), color(2), (sum[3] / 4) as u8));
        }
    }
    ColorImage::new(size, pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_clip_decodes_into_frames_that_draw_something() {
        for gif in [GREETING, WORKING, WAITING, DONE, ERROR] {
            let frames = decode(gif);
            assert!(frames.len() > 1);
            assert!(frames.windows(2).all(|pair| pair[0].1 < pair[1].1));
            for (image, _) in &frames {
                assert!(image.pixels.iter().any(|p| p.a() > 200));
            }
        }
    }
}
