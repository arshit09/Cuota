//! Clawd's pixel sprites and the Claude spark, drawn from the artwork that ships with the
//! Claude Code VS Code extension (`clawd.svg`, `claude-logo.svg`). The sprites are for the
//! tray icons, where he dances while a session works and sleeps otherwise; the panel's Clawd
//! is in `animations`.

use crate::vector;
use eframe::egui::{
    Color32, ColorImage, Context, Mesh, Painter, Pos2, Shape, TextureFilter, TextureHandle, TextureOptions,
    epaint::Vertex, pos2, vec2,
};

pub const CLAUDE_ORANGE: Color32 = Color32::from_rgb(217, 119, 87);
pub const EYE: Color32 = Color32::from_rgb(24, 24, 26);
pub const SNORE: Color32 = Color32::from_rgb(150, 160, 180);

/// Clawd on a grid of 11 columns by 8 rows: `clawd.svg` has 4 rows, split in half here because
/// his eyes are half-height gaps. '#' body, 'o' open eye, '-' closed eye.
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
/// (arms up in turn), breathing while asleep.
pub fn sprite(working: bool, frame: u32) -> (&'static [&'static str; 8], bool) {
    match (working, frame % 4) {
        (true, 1) => (&LEFT_ARM_UP, true),
        (true, 3) => (&RIGHT_ARM_UP, true),
        (true, _) => (&STAND, false),
        (false, f) if f % 2 == 1 => (&ASLEEP_INHALE, false),
        (false, _) => (&ASLEEP, false),
    }
}

/// The spark rasterized to `size` square RGBA: `SPARK_PIXELS` for the texture and the window
/// icon, or the tray's icon size.
pub fn spark_rgba(size: usize) -> Vec<u8> {
    let [r, g, b, _] = CLAUDE_ORANGE.to_array();
    vector::mask(SPARK_PATH, SPARK_VIEWBOX, size).into_iter().flat_map(|alpha| [r, g, b, alpha]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_spark_outline() {
        let outlines = vector::outlines(SPARK_PATH);
        assert_eq!(outlines.len(), 1);
        let outline = &outlines[0];
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

    /// Cuota's icon, in its exe and setup's (see build.rs) and installed for its shortcut, is
    /// Clawd at the sizes Windows asks for. After changing him, run this with CUOTA_UPDATE_ICON
    /// set to redraw `setup/cuota.ico`.
    #[test]
    fn the_icon_is_clawd() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/setup/cuota.ico");
        let drawn = ico(&[16, 20, 24, 32, 40, 48, 64, 256]);
        if std::env::var_os("CUOTA_UPDATE_ICON").is_some() {
            std::fs::write(path, &drawn).unwrap();
        }
        assert!(std::fs::read(path).unwrap() == drawn, "setup/cuota.ico is out of date");
    }

    /// An .ico of 32-bit bitmaps of Clawd, one per size.
    fn ico(sizes: &[usize]) -> Vec<u8> {
        let images: Vec<Vec<u8>> = sizes.iter().map(|&size| bitmap(&crate::tray::draw_icon(size), size)).collect();
        let mut ico = vec![0, 0, 1, 0];
        ico.extend((sizes.len() as u16).to_le_bytes());
        let mut offset = 6 + 16 * sizes.len();
        for (&size, image) in sizes.iter().zip(&images) {
            // 256 is written as 0.
            ico.extend([size as u8, size as u8, 0, 0]);
            ico.extend(1u16.to_le_bytes()); // planes
            ico.extend(32u16.to_le_bytes()); // bits per pixel
            ico.extend((image.len() as u32).to_le_bytes());
            ico.extend((offset as u32).to_le_bytes());
            offset += image.len();
        }
        images.iter().for_each(|image| ico.extend(image));
        ico
    }

    /// Square RGBA as an icon bitmap: a header twice as tall as the image, BGRA rows from the
    /// bottom up, then an all-clear 1-bit mask, which alpha makes unused.
    fn bitmap(rgba: &[u8], size: usize) -> Vec<u8> {
        let mut image = Vec::new();
        image.extend(40u32.to_le_bytes());
        image.extend((size as i32).to_le_bytes());
        image.extend((2 * size as i32).to_le_bytes());
        image.extend(1u16.to_le_bytes());
        image.extend(32u16.to_le_bytes());
        image.extend([0u8; 24]);
        for row in rgba.chunks_exact(size * 4).rev() {
            row.chunks_exact(4).for_each(|p| image.extend([p[2], p[1], p[0], p[3]]));
        }
        image.resize(image.len() + size.div_ceil(32) * 4 * size, 0);
        image
    }
}
