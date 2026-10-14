//! SVG path data filled into anti-aliased masks, for the marks Cuota draws itself rather than
//! with a font: the Claude spark and GitHub's mark.

use eframe::egui::{Vec2, emath::Rot2, vec2};
use std::f32::consts::{FRAC_PI_2, TAU};

/// Straight steps per Bézier curve, and per quarter turn of an arc.
const CURVE_STEPS: usize = 16;
/// Scanlines through each row of pixels. Along them, coverage is exact.
const SCANLINES: usize = 16;

/// How much of each pixel `path` covers, from 0 to 255, row by row, in a square `size` pixels
/// across. The path is drawn on a square viewBox `viewbox` units across, filled with SVG's
/// default nonzero rule.
pub fn mask(path: &str, viewbox: f32, size: usize) -> Vec<u8> {
    let scale = size as f32 / viewbox;
    // Each outline closes back to its first point, as a fill does.
    let edges: Vec<(Vec2, Vec2)> = outlines(path)
        .iter()
        .flat_map(|outline| outline.iter().zip(outline.iter().cycle().skip(1)))
        .map(|(&a, &b)| (a * scale, b * scale))
        .collect();

    let mut coverage = vec![0.0; size * size];
    let mut crossings: Vec<(f32, i32)> = Vec::new();
    for (row, pixels) in coverage.chunks_mut(size).enumerate() {
        for scanline in 0..SCANLINES {
            let y = row as f32 + (scanline as f32 + 0.5) / SCANLINES as f32;
            crossings.clear();
            for &(a, b) in &edges {
                if (a.y <= y) != (b.y <= y) {
                    let x = a.x + (y - a.y) / (b.y - a.y) * (b.x - a.x);
                    crossings.push((x, if b.y > a.y { 1 } else { -1 }));
                }
            }
            crossings.sort_by(|p, q| p.0.total_cmp(&q.0));
            // Between two crossings is inside wherever the outlines wind around it.
            let mut winding = 0;
            for pair in crossings.windows(2) {
                winding += pair[0].1;
                if winding != 0 {
                    cover(pixels, pair[0].0, pair[1].0, 1.0 / SCANLINES as f32);
                }
            }
        }
    }
    coverage.into_iter().map(|c| (c * 255.0).round() as u8).collect()
}

/// Adds `weight` times how much of each pixel of `row` lies between `left` and `right`.
fn cover(row: &mut [f32], left: f32, right: f32, weight: f32) {
    let width = row.len() as f32;
    let (mut x, right) = (left.clamp(0.0, width), right.clamp(0.0, width));
    while x < right {
        let pixel = x as usize;
        let end = right.min(pixel as f32 + 1.0);
        row[pixel] += (end - x) * weight;
        x = end;
    }
}

/// The outlines `path` draws, as polygons in its own units. It reads the commands the marks
/// use, M, L, H, V, C, S, A and Z in either case, and stops at any other.
pub fn outlines(path: &str) -> Vec<Vec<Vec2>> {
    let mut path = PathReader { bytes: path.as_bytes(), at: 0 };
    let (mut outlines, mut outline) = (Vec::new(), Vec::new());
    let (mut pen, mut start, mut command) = (Vec2::ZERO, Vec2::ZERO, b'M');
    // The last curve's second control point, which an S mirrors for its first.
    let mut last_control = None;
    while !path.done() {
        let at = path.at;
        if let Some(c) = path.command() {
            command = c;
        }
        let base = if command.is_ascii_lowercase() { pen } else { Vec2::ZERO };
        let mut control = None;
        match command.to_ascii_uppercase() {
            b'M' => {
                end_outline(&mut outlines, &mut outline);
                pen = base + path.point();
                start = pen;
                outline.push(pen);
                // Further pairs after a move are lines.
                command = if command == b'm' { b'l' } else { b'L' };
            }
            b'L' => {
                pen = base + path.point();
                outline.push(pen);
            }
            b'H' => {
                pen.x = base.x + path.number();
                outline.push(pen);
            }
            b'V' => {
                pen.y = base.y + path.number();
                outline.push(pen);
            }
            b'C' | b'S' => {
                let first = if command.eq_ignore_ascii_case(&b'C') {
                    base + path.point()
                } else {
                    last_control.map_or(pen, |c| pen * 2.0 - c)
                };
                let (second, end) = (base + path.point(), base + path.point());
                outline.extend(cubic(pen, first, second, end));
                (pen, control) = (end, Some(second));
            }
            b'A' => {
                let (radii, rotation, large, sweep) = (path.point(), path.number(), path.flag(), path.flag());
                let end = base + path.point();
                outline.extend(arc(pen, radii, rotation.to_radians(), large, sweep, end));
                pen = end;
            }
            b'Z' => {
                end_outline(&mut outlines, &mut outline);
                // Anything drawn next, short of a move, starts where this outline did.
                pen = start;
                outline.push(pen);
            }
            _ => break,
        }
        last_control = control;
        // Stop at anything unreadable rather than go round forever.
        if path.at == at {
            break;
        }
    }
    end_outline(&mut outlines, &mut outline);
    outlines
}

/// Keeps the outline drawn so far if it has an inside, and starts a new one.
fn end_outline(outlines: &mut Vec<Vec<Vec2>>, outline: &mut Vec<Vec2>) {
    let outline = std::mem::take(outline);
    if outline.len() > 2 {
        outlines.push(outline);
    }
}

/// A cubic Bézier curve in straight steps, from `from` (left out) to `to`.
fn cubic(from: Vec2, first: Vec2, second: Vec2, to: Vec2) -> impl Iterator<Item = Vec2> {
    (1..=CURVE_STEPS).map(move |step| {
        let t = step as f32 / CURVE_STEPS as f32;
        let u = 1.0 - t;
        u * u * u * from + 3.0 * u * u * t * first + 3.0 * u * t * t * second + t * t * t * to
    })
}

/// An elliptical arc in straight steps, from `from` (left out) to `to`, found from its ends as
/// the SVG spec's notes on implementing arcs describe.
fn arc(from: Vec2, radii: Vec2, rotation: f32, large: bool, sweep: bool, to: Vec2) -> Vec<Vec2> {
    let mut radii = radii.abs();
    // With nowhere to go or no radius, it's a straight line.
    if from == to || radii.x == 0.0 || radii.y == 0.0 {
        return vec![to];
    }
    // Halfway from `to` to `from`, along the ellipse's own axes.
    let (turn, unturn) = (Rot2::from_angle(rotation), Rot2::from_angle(-rotation));
    let half = unturn * ((from - to) / 2.0);
    // Radii too small to reach from one end to the other grow until they just do.
    let reach = (half.x / radii.x).powi(2) + (half.y / radii.y).powi(2);
    if reach > 1.0 {
        radii *= reach.sqrt();
    }
    let (rx, ry) = (radii.x, radii.y);
    let (rx2, ry2, hx2, hy2) = (rx * rx, ry * ry, half.x * half.x, half.y * half.y);
    let offset = ((rx2 * ry2 - rx2 * hy2 - ry2 * hx2) / (rx2 * hy2 + ry2 * hx2)).max(0.0).sqrt();
    let sign = if large == sweep { -1.0 } else { 1.0 };
    let center = sign * offset * vec2(rx * half.y / ry, -ry * half.x / rx);

    let angle = |v: Vec2| vec2(v.x / rx, v.y / ry).angle();
    let first = angle(half - center);
    let mut delta = angle(-half - center) - first;
    if sweep && delta < 0.0 {
        delta += TAU;
    } else if !sweep && delta > 0.0 {
        delta -= TAU;
    }
    let middle = turn * center + (from + to) / 2.0;
    let steps = (delta.abs() / FRAC_PI_2 * CURVE_STEPS as f32).ceil().max(1.0) as usize;
    (1..=steps)
        .map(|step| {
            let angle = first + delta * step as f32 / steps as f32;
            middle + turn * vec2(rx * angle.cos(), ry * angle.sin())
        })
        .collect()
}

struct PathReader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl PathReader<'_> {
    fn skip_separators(&mut self) {
        while self.bytes.get(self.at).is_some_and(|b| b.is_ascii_whitespace() || *b == b',') {
            self.at += 1;
        }
    }

    fn done(&mut self) -> bool {
        self.skip_separators();
        self.at == self.bytes.len()
    }

    fn command(&mut self) -> Option<u8> {
        self.skip_separators();
        let c = *self.bytes.get(self.at).filter(|b| b.is_ascii_alphabetic())?;
        self.at += 1;
        Some(c)
    }

    /// SVG numbers can run together: "-.23", or "1.008.656" for 1.008 then .656.
    fn number(&mut self) -> f32 {
        self.skip_separators();
        let start = self.at;
        if matches!(self.bytes.get(self.at), Some(b'-' | b'+')) {
            self.at += 1;
        }
        let mut seen_dot = false;
        while let Some(&b) = self.bytes.get(self.at) {
            match b {
                b'0'..=b'9' => {}
                b'.' if !seen_dot => seen_dot = true,
                _ => break,
            }
            self.at += 1;
        }
        std::str::from_utf8(&self.bytes[start..self.at]).ok().and_then(|s| s.parse().ok()).unwrap_or(0.0)
    }

    fn point(&mut self) -> Vec2 {
        vec2(self.number(), self.number())
    }

    /// Arc flags are single digits that may touch the next number: "01-.104".
    fn flag(&mut self) -> bool {
        self.skip_separators();
        self.at += 1;
        self.bytes.get(self.at - 1) == Some(&b'1')
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curves_and_arcs_end_where_their_commands_say() {
        // A relative curve, a smooth one mirroring it, then half a circle back to the start.
        let outlines = outlines("M0 0c1 0 2 1 2 2S3 4 4 4A2 2 0 0 1 0 0z");
        assert_eq!(outlines.len(), 1);
        let outline = &outlines[0];
        assert_eq!(outline[CURVE_STEPS], vec2(2.0, 2.0));
        assert_eq!(outline[2 * CURVE_STEPS], vec2(4.0, 4.0));
        // The smooth curve's first control point mirrors the first curve's second, (2, 1), to
        // (2, 3), which puts its middle at (2 + 3 * (2, 3) + 3 * (3, 4) + 4) / 8.
        let middle = outline[CURVE_STEPS + CURVE_STEPS / 2];
        assert!((middle - vec2(2.625, 3.375)).length() < 1e-5, "{middle:?}");
        // Too small to reach, the arc's radius grows to half the way back: a half circle around
        // (2, 2), swept clockwise on screen, through (0, 4).
        let arc = &outline[2 * CURVE_STEPS..];
        assert!(arc.iter().all(|p| ((*p - vec2(2.0, 2.0)).length() - 2.0 * 2.0f32.sqrt()).abs() < 1e-3));
        let middle = arc[arc.len() / 2];
        assert!((middle - vec2(0.0, 4.0)).length() < 1e-3, "{middle:?}");
        assert!(arc.last().unwrap().length() < 1e-3);
    }

    #[test]
    fn edges_cover_pixels_by_how_much_of_them_is_inside() {
        // A square from the middle of the first pixel to the middle of the last.
        let mask = mask("M.5.5H3.5V3.5H.5z", 4.0, 4);
        assert_eq!(&mask[0..4], &[64, 128, 128, 64]);
        assert_eq!(&mask[4..8], &[128, 255, 255, 128]);
    }

    #[test]
    fn githubs_mark_is_a_disc_with_the_octocat_cut_out() {
        let mask = mask(crate::widgets::GITHUB_MARK, 16.0, 32);
        let at = |x: usize, y: usize| mask[y * 32 + x];
        // Clear past the disc's round edge, solid within it.
        for (x, y) in [(0, 0), (31, 0), (0, 31), (31, 31)] {
            assert_eq!(at(x, y), 0, "corner ({x}, {y})");
        }
        for (x, y) in [(16, 1), (2, 16), (29, 16)] {
            assert_eq!(at(x, y), 255, "disc at ({x}, {y})");
        }
        // The Octocat's head, and its body under it down to the disc's foot, are holes.
        for (x, y) in [(16, 14), (8, 16), (24, 16), (16, 24), (16, 30)] {
            assert_eq!(at(x, y), 0, "Octocat at ({x}, {y})");
        }
    }
}
