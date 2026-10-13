//! Clawd and the Claude spark, drawn from the artwork that ships with the Claude Code VS Code
//! extension (`clawd.svg`, `claude-logo.svg`). While a Claude session works, the spark orbits
//! Clawd and he dances, following it with his eyes; when every session is idle he sleeps.

use eframe::egui::{
    Color32, ColorImage, Context, Mesh, Painter, Pos2, Rect, Shape, TextureFilter, TextureHandle,
    TextureOptions, Vec2, epaint::Vertex, pos2, vec2,
};
use std::f32::consts::{PI, TAU};
use std::time::Duration;

pub const CLAUDE_ORANGE: Color32 = Color32::from_rgb(217, 119, 87);
pub const EYE: Color32 = Color32::from_rgb(24, 24, 26);
pub const SNORE: Color32 = Color32::from_rgb(150, 160, 180);

/// One cell of Clawd's grid. `clawd.svg` is 11 columns by 4 rows of 4.15 x 9 units; rows are
/// split in half here because his eyes are half-height gaps.
const CELL: Vec2 = vec2(4.0, 4.5);
const SPRITE: Vec2 = vec2(11.0 * CELL.x, 8.0 * CELL.y);
/// Room around him for the hop and the spark's orbit, at scale 1.
const CANVAS: Vec2 = vec2(76.0, 54.0);
const HOP: f32 = CELL.y;
const ORBIT_CENTER: Vec2 = vec2(CANVAS.x / 2.0, 20.0);
const ORBIT_RADIUS: Vec2 = vec2(29.0, 10.0);
const SPARK_RADIUS: f32 = 9.0;

pub const STAND: [&str; 8] = [
    ".#########.",
    ".#########.",
    "##o#####o##",
    "###########",
    ".#########.",
    ".#########.",
    ".#.#...#.#.",
    ".#.#...#.#.",
];
const LEFT_ARM_UP: [&str; 8] = [
    "##########.",
    "##########.",
    ".#o#####o#.",
    ".#########.",
    ".##########",
    ".##########",
    ".#.#...#.#.",
    ".#.#...#.#.",
];
const RIGHT_ARM_UP: [&str; 8] = [
    ".##########",
    ".##########",
    ".#o#####o#.",
    ".#########.",
    "##########.",
    "##########.",
    ".#.#...#.#.",
    ".#.#...#.#.",
];
const ASLEEP: [&str; 8] = [
    "...........",
    "...........",
    ".#########.",
    ".#########.",
    "##-#####-##",
    "###########",
    ".#########.",
    ".#########.",
];
const ASLEEP_INHALE: [&str; 8] = [
    "...........",
    "..#######..",
    ".#########.",
    ".#########.",
    "##-#####-##",
    "###########",
    ".#########.",
    ".#########.",
];
const SNORE_Z: [&str; 5] = ["#####", "...#.", "..#..", ".#...", "#####"];

/// The path from `claude-logo.svg` (24 x 24 viewBox).
const SPARK_PATH: &str = "M4.709 15.955l4.72-2.647.08-.23-.08-.128H9.2l-.79-.048-2.698-.073-2.339-.097-2.266-.122-.571-.121L0 11.784l.055-.352.48-.321.686.06 1.52.103 2.278.158 1.652.097 2.449.255h.389l.055-.157-.134-.098-.103-.097-2.358-1.596-2.552-1.688-1.336-.972-.724-.491-.364-.462-.158-1.008.656-.722.881.06.225.061.893.686 1.908 1.476 2.491 1.833.365.304.145-.103.019-.073-.164-.274-1.355-2.446-1.446-2.49-.644-1.032-.17-.619a2.97 2.97 0 01-.104-.729L6.283.134 6.696 0l.996.134.42.364.62 1.414 1.002 2.229 1.555 3.03.456.898.243.832.091.255h.158V9.01l.128-1.706.237-2.095.23-2.695.08-.76.376-.91.747-.492.584.28.48.685-.067.444-.286 1.851-.559 2.903-.364 1.942h.212l.243-.242.985-1.306 1.652-2.064.73-.82.85-.904.547-.431h1.033l.76 1.129-.34 1.166-1.064 1.347-.881 1.142-1.264 1.7-.79 1.36.073.11.188-.02 2.856-.606 1.543-.28 1.841-.315.833.388.091.395-.328.807-1.969.486-2.309.462-3.439.813-.042.03.049.061 1.549.146.662.036h1.622l3.02.225.79.522.474.638-.079.485-1.215.62-1.64-.389-3.829-.91-1.312-.329h-.182v.11l1.093 1.068 2.006 1.81 2.509 2.33.127.578-.322.455-.34-.049-2.205-1.657-.851-.747-1.926-1.62h-.128v.17l.444.649 2.345 3.521.122 1.08-.17.353-.608.213-.668-.122-1.374-1.925-1.415-2.167-1.143-1.943-.14.08-.674 7.254-.316.37-.729.28-.607-.461-.322-.747.322-1.476.389-1.924.315-1.53.286-1.9.17-.632-.012-.042-.14.018-1.434 1.967-2.18 2.945-1.726 1.845-.414.164-.717-.37.067-.662.401-.589 2.388-3.036 1.44-1.882.93-1.086-.006-.158h-.055L4.132 18.56l-1.13.146-.487-.456.061-.746.231-.243 1.908-1.312-.006.006z";
const SPARK_VIEWBOX: f32 = 24.0;
pub const SPARK_PIXELS: usize = 64;

pub struct Mascot {
    spark: TextureHandle,
}

impl Mascot {
    pub fn new(ctx: &Context, spark_rgba: &[u8]) -> Self {
        let image = ColorImage::from_rgba_unmultiplied([SPARK_PIXELS, SPARK_PIXELS], spark_rgba);
        // Mipmaps keep the rays crisp when the spark is drawn far smaller than the texture.
        let options = TextureOptions { mipmap_mode: Some(TextureFilter::Linear), ..TextureOptions::LINEAR };
        Self { spark: ctx.load_texture("claude-spark", image, options) }
    }

    pub fn size(scale: f32) -> Vec2 {
        CANVAS * scale
    }

    /// How long the current animation frame lasts.
    pub fn frame_time(working: bool) -> Duration {
        Duration::from_millis(if working { 83 } else { 166 })
    }

    /// Paints Clawd into `rect` (sized by [`Self::size`]). A fixed `time` gives a still frame.
    pub fn paint(&self, painter: &Painter, rect: Rect, working: bool, time: f64) {
        let scale = rect.width() / CANVAS.x;
        let at = |x: f32, y: f32| rect.left_top() + vec2(x, y) * scale;
        let time = time as f32;

        if !working {
            let (asleep, _) = sprite(false, (time / 1.6) as u32);
            paint_sprite(painter, at(0.0, 0.0), scale, asleep, 0.0, Vec2::ZERO);
            for i in 0..3 {
                let phase = (time / 3.0 + i as f32 / 3.0).fract();
                let pixel = (0.8 + phase * 0.8) * scale;
                let color = SNORE.gamma_multiply((phase * PI).sin());
                paint_grid(painter, at(56.0 + phase * 14.0, 22.0 - phase * 20.0), vec2(pixel, pixel), &SNORE_Z, color);
            }
            return;
        }

        // The spark circles his head; it is behind him on the upper half of the orbit.
        let angle = time * 1.6 % TAU;
        let (sin, cos) = angle.sin_cos();
        let spark_at = at(ORBIT_CENTER.x + ORBIT_RADIUS.x * cos, ORBIT_CENTER.y + ORBIT_RADIUS.y * sin);
        let behind = sin < 0.0;
        let spark_radius = SPARK_RADIUS * scale * if behind { 0.75 } else { 1.0 };
        let spin = time * 2.0;
        // Eyes follow the spark: up to half a column sideways, a little up or down.
        let gaze = vec2((cos * 2.0).round(), (sin * 1.5).round()) * scale;

        let (dancing, hop) = sprite(true, (time / 0.3) as u32);
        if behind {
            self.paint_spark(painter, spark_at, spark_radius, spin, Color32::from_white_alpha(200));
        }
        paint_sprite(painter, at(0.0, 0.0), scale, dancing, if hop { HOP } else { 0.0 }, gaze);
        if !behind {
            self.paint_spark(painter, spark_at, spark_radius, spin, Color32::WHITE);
        }
    }

    /// The Claude spark on its own, upright.
    pub fn paint_logo(&self, painter: &Painter, center: Pos2, radius: f32) {
        self.paint_spark(painter, center, radius, 0.0, Color32::WHITE);
    }

    fn paint_spark(&self, painter: &Painter, center: Pos2, radius: f32, angle: f32, tint: Color32) {
        let (sin, cos) = angle.sin_cos();
        let corner = |x: f32, y: f32| center + vec2(x * cos - y * sin, x * sin + y * cos) * radius;
        let mut mesh = Mesh::with_texture(self.spark.id());
        for (x, y, u, v) in [(-1.0, -1.0, 0.0, 0.0), (1.0, -1.0, 1.0, 0.0), (1.0, 1.0, 1.0, 1.0), (-1.0, 1.0, 0.0, 1.0)] {
            mesh.vertices.push(Vertex { pos: corner(x, y), uv: pos2(u, v), color: tint });
        }
        mesh.indices.extend([0, 1, 2, 0, 2, 3]);
        painter.add(Shape::mesh(mesh));
    }
}

/// Clawd's pose for animation step `frame`, and whether he is mid-hop: dancing while working
/// (arms up in turn), breathing while asleep. '#' body, 'o' open eye, '-' closed eye.
pub fn sprite(working: bool, frame: u32) -> (&'static [&'static str; 8], bool) {
    match (working, frame % 4) {
        (true, 1) => (&LEFT_ARM_UP, true),
        (true, 3) => (&RIGHT_ARM_UP, true),
        (true, _) => (&STAND, false),
        (false, f) if f % 2 == 1 => (&ASLEEP_INHALE, false),
        (false, _) => (&ASLEEP, false),
    }
}

/// Clawd standing on the canvas floor, `hop` points up, eyes shifted by `gaze`.
fn paint_sprite(painter: &Painter, canvas: Pos2, scale: f32, sprite: &[&str; 8], hop: f32, gaze: Vec2) {
    let left = (CANVAS.x - SPRITE.x) / 2.0;
    let origin = canvas + vec2(left, CANVAS.y - SPRITE.y - hop) * scale;
    let cell = CELL * scale;
    paint_grid(painter, origin, cell, sprite, CLAUDE_ORANGE);

    let mut eyes = Mesh::default();
    for (row, line) in sprite.iter().enumerate() {
        for (col, pixel) in line.bytes().enumerate() {
            let socket = Rect::from_min_size(origin + vec2(col as f32 * cell.x, row as f32 * cell.y), cell);
            match pixel {
                b'o' => eyes.add_colored_rect(socket.translate(gaze), EYE),
                b'-' => {
                    let lid = Rect::from_min_max(pos2(socket.left(), socket.bottom() - 1.5 * scale), socket.right_bottom());
                    eyes.add_colored_rect(lid, EYE);
                }
                _ => {}
            }
        }
    }
    painter.add(Shape::mesh(eyes));
}

/// Fills every non-'.' cell of `grid` as one mesh: separately anti-aliased rects leave seams.
fn paint_grid(painter: &Painter, origin: Pos2, cell: Vec2, grid: &[&str], color: Color32) {
    let mut mesh = Mesh::default();
    for (row, line) in grid.iter().enumerate() {
        for (col, _) in line.bytes().enumerate().filter(|&(_, b)| b != b'.') {
            let min = origin + vec2(col as f32 * cell.x, row as f32 * cell.y);
            mesh.add_colored_rect(Rect::from_min_size(min, cell), color);
        }
    }
    painter.add(Shape::mesh(mesh));
}

/// The spark rasterized to `size` square RGBA: `SPARK_PIXELS` for the texture and the window
/// icon, or the tray's icon size.
pub fn spark_rgba(size: usize) -> Vec<u8> {
    const SAMPLES: usize = 4;
    let scale = (size * SAMPLES) as f32 / SPARK_VIEWBOX;
    let outline: Vec<Vec2> = spark_outline().into_iter().map(|p| p * scale).collect();
    let edges: Vec<(Vec2, Vec2)> = outline.iter().zip(outline.iter().cycle().skip(1)).map(|(&a, &b)| (a, b)).collect();

    // Scanline fill with the nonzero rule, one scanline per sample row.
    let mut coverage = vec![0u16; size * size];
    let mut crossings: Vec<(f32, i32)> = Vec::new();
    for sample_row in 0..size * SAMPLES {
        let y = sample_row as f32 + 0.5;
        crossings.clear();
        for &(a, b) in &edges {
            if (a.y <= y) != (b.y <= y) {
                let x = a.x + (y - a.y) / (b.y - a.y) * (b.x - a.x);
                crossings.push((x, if b.y > a.y { 1 } else { -1 }));
            }
        }
        crossings.sort_by(|p, q| p.0.total_cmp(&q.0));
        let (mut winding, mut next) = (0, 0);
        for sample_col in 0..size * SAMPLES {
            let x = sample_col as f32 + 0.5;
            while next < crossings.len() && crossings[next].0 < x {
                winding += crossings[next].1;
                next += 1;
            }
            if winding != 0 {
                coverage[sample_row / SAMPLES * size + sample_col / SAMPLES] += 1;
            }
        }
    }

    let [r, g, b, _] = CLAUDE_ORANGE.to_array();
    let full = (SAMPLES * SAMPLES) as u32;
    coverage.iter().flat_map(|&c| [r, g, b, (u32::from(c) * 255 / full) as u8]).collect()
}

/// The outline of `SPARK_PATH`. It only uses straight segments and one tiny arc, which is
/// close enough to straight at icon sizes.
fn spark_outline() -> Vec<Vec2> {
    let mut path = PathReader { bytes: SPARK_PATH.as_bytes(), at: 0 };
    let (mut points, mut pen, mut command) = (Vec::new(), Vec2::ZERO, b'M');
    while !path.done() {
        if let Some(c) = path.command() {
            command = c;
        }
        let base = if command.is_ascii_lowercase() { pen } else { Vec2::ZERO };
        match command.to_ascii_uppercase() {
            b'M' | b'L' => {
                pen = base + vec2(path.number(), path.number());
                // Further pairs after a move are lines.
                command = if command == b'm' { b'l' } else if command == b'M' { b'L' } else { command };
            }
            b'H' => pen.x = base.x + path.number(),
            b'V' => pen.y = base.y + path.number(),
            b'A' => {
                // Radii, rotation and the two flags only shape the curve; keep its endpoint.
                for _ in 0..3 {
                    path.number();
                }
                path.flag();
                path.flag();
                pen = base + vec2(path.number(), path.number());
            }
            _ => break,
        }
        points.push(pen);
    }
    points
}

struct PathReader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl PathReader<'_> {
    fn skip_separators(&mut self) {
        while self.bytes.get(self.at).is_some_and(|b| matches!(b, b' ' | b',')) {
            self.at += 1;
        }
    }

    fn done(&mut self) -> bool {
        self.skip_separators();
        // 'z' closes the outline, which `spark_rgba` does anyway.
        self.bytes.get(self.at).is_none_or(|b| b.eq_ignore_ascii_case(&b'z'))
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
    fn reads_the_spark_outline() {
        let outline = spark_outline();
        assert!(outline.len() > 150, "got {} points", outline.len());
        assert_eq!(outline[0], vec2(4.709, 15.955));
        assert!(outline.iter().all(|p| (-0.01..=24.01).contains(&p.x) && (-0.01..=24.01).contains(&p.y)));
        // The path closes back where it started.
        assert!((*outline.last().unwrap() - outline[0]).length() < 0.05);
    }

    #[test]
    fn spark_covers_a_reasonable_area() {
        let rgba = spark_rgba(SPARK_PIXELS);
        let opaque = rgba.chunks(4).filter(|p| p[3] > 128).count();
        let share = opaque as f32 / (SPARK_PIXELS * SPARK_PIXELS) as f32;
        assert!((0.15..0.5).contains(&share), "spark covers {share}");
    }
}
