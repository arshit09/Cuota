//! Building blocks for the settings window, made to look and work like Windows 11's own Settings:
//! sidebar entries for its pages, setting cards (an icon, a title, an optional description and a
//! control), expanders that open onto more settings, and the controls themselves. The expanded
//! panel's details share the section headings, cards and rules, drawn in the panel's colors.

use crate::mascot::{self, CLAUDE_ORANGE};
use crate::theme::Palette;
use crate::vector;
use eframe::egui::{
    Align, Align2, Color32, ColorImage, Context, CornerRadius, CursorIcon, EventFilter, FontFamily, FontId, Frame,
    Galley, Id, Key, Layout, Margin, Modifiers, Painter, Popup, Pos2, Rect, Response, Sense, Shadow, Shape, Stroke,
    StrokeKind, TextureHandle, TextureId, TextureOptions, Ui, UiBuilder, Vec2, WidgetInfo, WidgetType, emath,
    emath::GuiRounding, pos2,
    style::ScrollAnimation,
    text::{LayoutJob, TextWrapping},
    vec2,
};
use std::ops::RangeInclusive;
use std::sync::Arc;

// Windows 11's dark theme, which the settings window follows.
pub const WINDOW: Color32 = Color32::from_rgb(32, 32, 32);
pub const PRIMARY: Color32 = Color32::WHITE;
pub const SECONDARY: Color32 = Color32::from_rgb(208, 208, 208);
/// A setting's card, and its edge.
const SETTING: Color32 = Color32::from_rgb(43, 43, 43);
const SETTING_HOVER: Color32 = Color32::from_rgb(50, 50, 50);
const SETTING_EDGE: Color32 = Color32::from_rgb(29, 29, 29);
/// Under an open expander, where its settings go.
const INSET: Color32 = Color32::from_rgb(39, 39, 39);
/// A sidebar entry that's picked or under the pointer.
const SUBTLE: Color32 = Color32::from_rgb(45, 45, 45);
const SUBTLE_PRESSED: Color32 = Color32::from_rgb(41, 41, 41);
/// Buttons and dropdowns.
const CONTROL: Color32 = Color32::from_rgb(56, 56, 56);
const CONTROL_HOVER: Color32 = Color32::from_rgb(61, 61, 61);
const CONTROL_PRESSED: Color32 = Color32::from_rgb(50, 50, 50);
const CONTROL_EDGE: Color32 = Color32::from_rgb(64, 64, 64);
/// Switches and check boxes while off.
const OFF: Color32 = Color32::from_rgb(39, 39, 39);
const OFF_HOVER: Color32 = Color32::from_rgb(52, 52, 52);
/// Their outlines, and a slider's empty track.
const STRONG: Color32 = Color32::from_rgb(158, 158, 158);
const THUMB: Color32 = Color32::from_rgb(69, 69, 69);
const POPUP: Color32 = Color32::from_rgb(44, 44, 44);
const POPUP_EDGE: Color32 = Color32::from_rgb(64, 64, 64);
const ACCENT: Color32 = CLAUDE_ORANGE;
/// Windows' dark theme draws black on its accent color.
const ON_ACCENT: Color32 = Color32::BLACK;

/// Glyphs of Segoe Fluent Icons (Windows 11), which keeps Segoe MDL2 Assets' (Windows 10) code points.
pub mod icon {
    pub const ANIMATE: char = '\u{E768}';
    pub const APPEARANCE: char = '\u{E771}';
    /// A pulse on a chart: what tasks take on average.
    pub const AVERAGE: char = '\u{E9D9}';
    pub const BROWSE: char = '\u{E8E5}';
    pub const CHECK: char = '\u{E73E}';
    pub const CHECKLIST: char = '\u{E9D5}';
    pub const CHEVRON_DOWN: char = '\u{E70D}';
    pub const CHEVRON_UP: char = '\u{E70E}';
    pub const CLAWD: char = '\u{E76E}';
    pub const CLEAR: char = '\u{E74D}';
    /// A head and shoulders: an account with no letter to show for it.
    pub const CONTACT: char = '\u{E77B}';
    pub const DETAILS: char = '\u{E9D2}';
    pub const DOUBLE_CLICK: char = '\u{E962}';
    pub const DOWNLOAD: char = '\u{E896}';
    /// Leads out of the app, as to a web page.
    pub const EXTERNAL: char = '\u{E8A7}';
    pub const FOLDER: char = '\u{E838}';
    /// The web: a website.
    pub const GLOBE: char = '\u{E774}';
    pub const HEAR: char = '\u{E7F6}';
    /// A clock with an arrow round it: what's been.
    pub const HISTORY: char = '\u{E81C}';
    /// A card with a picture and lines.
    pub const HOVER_CARD: char = '\u{E8A1}';
    /// An eye: see which monitor is which.
    pub const IDENTIFY: char = '\u{E7B3}';
    /// An i in a circle: about the app.
    pub const INFO: char = '\u{E946}';
    pub const LIMITS: char = '\u{E9F9}';
    pub const LOG: char = '\u{E8A5}';
    /// Two screens side by side.
    pub const MONITORS: char = '\u{EBC6}';
    pub const MOVE_ASIDE: char = '\u{E759}';
    pub const OPACITY: char = '\u{E793}';
    pub const PANEL: char = '\u{E737}';
    pub const PERCENT: char = '\u{E94C}';
    pub const PLAY: char = '\u{E768}';
    /// A pointer, touched.
    pub const REACT: char = '\u{E7C9}';
    pub const QUIT: char = '\u{E7E8}';
    pub const REFRESH: char = '\u{E72C}';
    /// A gripper, to drag a row into another order by.
    pub const REORDER: char = '\u{E76F}';
    pub const SETTINGS: char = '\u{E713}';
    pub const SHOW_AS: char = '\u{E7F4}';
    /// A door with an arrow leaving it.
    pub const SIGN_OUT: char = '\u{F3B1}';
    pub const SIZE: char = '\u{E740}';
    pub const SOUND: char = '\u{E767}';
    pub const SOUND_MODE: char = '\u{E9E9}';
    /// A window coming up: Cuota opening by itself as Windows starts.
    pub const STARTUP: char = '\u{E78B}';
    pub const STATUS: char = '\u{E8BD}';
    pub const STYLE: char = '\u{EB05}';
    pub const TASKBAR: char = '\u{E75B}';
    /// A painter's palette, as on Windows' own Colors page, where light and dark mode are picked.
    pub const THEME: char = '\u{E790}';
    pub const TIME_LEFT: char = '\u{E916}';
    /// Arrows either way: off to another monitor and back.
    pub const TRAVEL: char = '\u{E8AB}';
    /// Windows Update's arrows going round.
    pub const UPDATE: char = '\u{E895}';
    pub const WHEEL: char = '\u{ECE7}';
}

/// Windows' type ramp: body text and controls, and captions under them.
const BODY: f32 = 14.0;
const CAPTION: f32 = 12.0;
const ICON_SIZE: f32 = 18.0;
/// A sidebar entry's icon.
const NAV_ICON: f32 = 16.0;
/// From a card's icon to its title.
const INDENT: f32 = 36.0;
const CARD_PADDING: f32 = 16.0;
const CARD_PADDING_Y: f32 = 12.0;
const CARD_HEIGHT: f32 = 62.0;
/// Between a card's text and its control.
const CONTROL_GAP: f32 = 16.0;
/// Narrower text than this and a card puts its control under the text, as Windows does.
const NARROWEST_TEXT: f32 = 140.0;
const CONTROL_HEIGHT: f32 = 32.0;
const SWITCH: [f32; 2] = [40.0, 20.0];
/// Room for a switch and the On or Off beside it.
pub const TOGGLE_WIDTH: f32 = SWITCH[0] + 10.0 + 24.0;
const CHECK_BOX: f32 = 20.0;
/// A dropdown is at least this wide, and keeps this much room for its chevron.
const DROPDOWN_WIDTH: f32 = 120.0;
const CHEVRON_ROOM: f32 = 34.0;
/// An expander's chevron.
const CHEVRON: f32 = 20.0;
/// How long a sortable list's rows take to slide out of a dragged row's way, in seconds.
const SLIDE: f32 = 0.12;
/// A sortable row's gripper lane when there's no icon column for it.
const GRIP_LANE: f32 = 28.0;
/// A dragged row scrolls its list this fast, in points a second, while the pointer is this
/// near the top or bottom of what shows of it.
const EDGE_SCROLL: f32 = 600.0;
const EDGE: f32 = 32.0;

/// The font families the UI draws with.
#[derive(Clone)]
pub struct Fonts {
    /// Semibold, for emphasis.
    pub strong: FontFamily,
    /// Windows' icon font. Without it, cards go without icons.
    pub icons: Option<FontFamily>,
}

impl Fonts {
    fn indent(&self) -> f32 {
        if self.icons.is_some() { INDENT } else { 0.0 }
    }

    fn icon(&self, size: f32) -> Option<FontId> {
        self.icons.clone().map(|family| FontId::new(size, family))
    }
}

/// A small caps section title, with an optional control on the right.
pub fn heading(ui: &mut Ui, fonts: &Fonts, palette: &Palette, text: &str, add_control: impl FnOnce(&mut Ui)) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
    let font = FontId::new(11.0, fonts.strong.clone());
    let mut job = LayoutJob::simple_singleline(text.to_uppercase(), font, palette.dim);
    job.sections[0].format.extra_letter_spacing = 0.6;
    let galley = ui.painter().layout_job(job);
    let at = pos2(rect.left() + 4.0, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(at, galley, palette.dim);
    let mut slot = ui.new_child(UiBuilder::new().max_rect(rect).layout(Layout::right_to_left(Align::Center)));
    add_control(&mut slot);
}

/// A rounded group of rows, with a [`rule`] between them.
pub fn card<R>(ui: &mut Ui, palette: &Palette, add_rows: impl FnOnce(&mut Ui) -> R) -> R {
    Frame::new()
        .fill(palette.card)
        .stroke(Stroke::new(1.0, palette.line))
        .corner_radius(8)
        .inner_margin(Margin::symmetric(14, 0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            add_rows(ui)
        })
        .inner
}

/// A hairline between a [`card`]'s rows, running to its edges.
pub fn rule(ui: &mut Ui, palette: &Palette) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(rect.left()..=rect.right() + 13.0, rect.center().y, Stroke::new(1.0, palette.line));
}

/// What a sidebar entry or a [`link_card`] shows before its name.
#[derive(Clone, Copy)]
pub enum NavIcon {
    /// A glyph of the icon font.
    Glyph(char),
    /// A white image as many pixels across as the glyphs, such as [`github_mark`], [`camera_mark`] or
    /// [`clawd_mark`], drawn in their color.
    Image(TextureId),
}

impl From<char> for NavIcon {
    fn from(glyph: char) -> Self {
        Self::Glyph(glyph)
    }
}

impl From<TextureId> for NavIcon {
    fn from(texture: TextureId) -> Self {
        Self::Image(texture)
    }
}

/// A page's entry in a sidebar, like in Windows' own Settings: its icon and name, filled in
/// and marked with an accent bar while its page shows.
pub fn nav_item(ui: &mut Ui, fonts: &Fonts, icon: impl Into<NavIcon>, name: &str, selected: bool) -> Response {
    let (_, response) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click());
    response.widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, ui.is_enabled(), selected, name));
    paint_nav_entry(ui, fonts, &response, icon.into(), name, selected);
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// The account at the foot of a sidebar, like the one in Windows' Start menu: its [`avatar`],
/// then `name` with `caption` under it, each on one line and cut short when too long. Lit while
/// `open`, as its menu is.
pub fn account_item(
    ui: &mut Ui,
    fonts: &Fonts,
    initial: Option<char>,
    name: &str,
    caption: Option<&str>,
    open: bool,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ACCOUNT_HEIGHT), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), name));
    let fill = match (response.hovered() || open, response.is_pointer_button_down_on()) {
        (_, true) => SUBTLE_PRESSED,
        (true, false) => SUBTLE,
        (false, false) => Color32::TRANSPARENT,
    };
    let painter = ui.painter();
    painter.rect_filled(rect, 4, fill);
    // Centered where the entries above have their icons, its text lined up with their names.
    avatar(painter, fonts, pos2(rect.left() + 14.0 + NAV_ICON / 2.0, rect.center().y), 16.0, initial);
    let text_left = rect.left() + 48.0;
    let room = rect.right() - 8.0 - text_left;
    let name = single_line(painter, name, FontId::proportional(BODY), PRIMARY, room);
    let caption = caption.map(|caption| single_line(painter, caption, FontId::proportional(CAPTION), SECONDARY, room));
    let height = name.size().y + caption.as_ref().map_or(0.0, |caption| caption.size().y);
    let mut at = pos2(text_left, rect.center().y - height / 2.0);
    let name_height = name.size().y;
    painter.galley(at, name, PRIMARY);
    if let Some(caption) = caption {
        at.y += name_height;
        painter.galley(at, caption, SECONDARY);
    }
    focus_ring(ui, &response, rect, 4.0);
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// How tall an [`account_item`] is.
pub const ACCOUNT_HEIGHT: f32 = 52.0;

/// A round picture for an account, as Windows draws one without a photo: `initial` in black on
/// the accent color, or a head and shoulders when there's no letter for it.
pub fn avatar(painter: &Painter, fonts: &Fonts, center: Pos2, radius: f32, initial: Option<char>) {
    match initial {
        Some(initial) => {
            painter.circle_filled(center, radius, ACCENT);
            let font = FontId::new(radius, fonts.strong.clone());
            painter.text(center, Align2::CENTER_CENTER, initial, font, ON_ACCENT);
        }
        None => {
            painter.circle(center, radius, CONTROL, Stroke::new(1.0, CONTROL_EDGE));
            if let Some(font) = fonts.icon(radius) {
                painter.text(center, Align2::CENTER_CENTER, icon::CONTACT, font, SECONDARY);
            }
        }
    }
}

/// A sidebar entry's fill, accent bar, icon and name.
fn paint_nav_entry(ui: &Ui, fonts: &Fonts, response: &Response, icon: NavIcon, name: &str, selected: bool) {
    let rect = response.rect;
    let fill = match (selected, response.hovered(), response.is_pointer_button_down_on()) {
        (_, _, true) | (true, true, _) => SUBTLE_PRESSED,
        (true, false, _) | (false, true, _) => SUBTLE,
        (false, false, _) => Color32::TRANSPARENT,
    };
    let painter = ui.painter();
    painter.rect_filled(rect, 4, fill);
    if selected {
        let bar = Rect::from_center_size(pos2(rect.left() + 1.5, rect.center().y), vec2(3.0, 16.0));
        painter.rect_filled(bar, 1.5, ACCENT);
    }
    let left = rect.left() + 14.0;
    paint_nav_icon(ui, fonts, icon, pos2(left + NAV_ICON / 2.0, rect.center().y), NAV_ICON);
    let text_left = if fonts.icons.is_some() { left + 34.0 } else { left };
    painter.text(pos2(text_left, rect.center().y), Align2::LEFT_CENTER, name, FontId::proportional(BODY), PRIMARY);
    focus_ring(ui, response, rect, 4.0);
}

/// A [`NavIcon`] centered on `center`, a glyph `size` points tall or an image as wide as a
/// sidebar entry's. Without the icon font nothing is drawn, images included, so the text
/// beside icons lines up.
fn paint_nav_icon(ui: &Ui, fonts: &Fonts, icon: NavIcon, center: Pos2, size: f32) {
    let Some(font) = fonts.icon(size) else { return };
    match icon {
        NavIcon::Glyph(glyph) => {
            ui.painter().text(center, Align2::CENTER_CENTER, glyph, font, PRIMARY);
        }
        NavIcon::Image(texture) => {
            // On whole pixels, a texel to each, so it stays sharp.
            let ppp = ui.pixels_per_point();
            let side = Vec2::splat(nav_icon_pixels(ppp) as f32 / ppp);
            let at = Rect::from_min_size((center - side / 2.0).round_to_pixels(ppp), side);
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            ui.painter().image(texture, at, uv, PRIMARY);
        }
    }
}

/// How many pixels across a sidebar entry's icon is.
fn nav_icon_pixels(pixels_per_point: f32) -> usize {
    (NAV_ICON * pixels_per_point).round().max(1.0) as usize
}

/// GitHub's mark: `mark-github` from GitHub's Primer Octicons (MIT), on a 16 x 16 viewBox.
pub const GITHUB_MARK: &str = "M6.766 11.328c-2.063-.25-3.516-1.734-3.516-3.656 0-.781.281-1.625.75-2.188-.203-.515-.172-1.609.063-2.062.625-.078 1.468.25 1.968.703.594-.187 1.219-.281 1.985-.281.765 0 1.39.094 1.953.265.484-.437 1.344-.765 1.969-.687.218.422.25 1.515.046 2.047.5.593.766 1.39.766 2.203 0 1.922-1.453 3.375-3.547 3.64.531.344.89 1.094.89 1.954v1.625c0 .468.391.734.86.547C13.781 14.359 16 11.53 16 8.03 16 3.61 12.406 0 7.984 0 3.563 0 0 3.61 0 8.031a7.88 7.88 0 0 0 5.172 7.422c.422.156.828-.125.828-.547v-1.25c-.219.094-.5.156-.75.156-1.031 0-1.64-.562-2.078-1.609-.172-.422-.36-.672-.719-.719-.187-.015-.25-.093-.25-.187 0-.188.313-.328.625-.328.453 0 .844.281 1.25.86.313.452.64.655 1.031.655s.641-.14 1-.5c.266-.265.47-.5.657-.656";

/// GitHub's mark as a [`NavIcon::Image`].
pub fn github_mark(ctx: &Context) -> TextureId {
    nav_image(ctx, "github-mark", |pixels| vector::mask(GITHUB_MARK, 16.0, pixels))
}

/// A camera, on a 16 x 16 viewBox: a rounded square with a round lens and a flash, the kind of
/// glyph that stands for Instagram. Drawn here, not taken from its logo. Each hole winds the
/// other way round from what it's cut out of.
pub const CAMERA_MARK: &str = "M4.75 .75H11.25A4 4 0 0 1 15.25 4.75V11.25A4 4 0 0 1 11.25 15.25H4.75A4 4 0 0 1 .75 11.25V4.75A4 4 0 0 1 4.75 .75Z\
    M4.75 2.25A2.5 2.5 0 0 0 2.25 4.75V11.25A2.5 2.5 0 0 0 4.75 13.75H11.25A2.5 2.5 0 0 0 13.75 11.25V4.75A2.5 2.5 0 0 0 11.25 2.25Z\
    M8 4.25A3.75 3.75 0 0 1 8 11.75A3.75 3.75 0 0 1 8 4.25Z\
    M8 5.75A2.25 2.25 0 0 0 8 10.25A2.25 2.25 0 0 0 8 5.75Z\
    M12 3A1 1 0 0 1 12 5A1 1 0 0 1 12 3Z";

/// [`CAMERA_MARK`] as a [`NavIcon::Image`].
pub fn camera_mark(ctx: &Context) -> TextureId {
    nav_image(ctx, "camera-mark", |pixels| vector::mask(CAMERA_MARK, 16.0, pixels))
}

/// Clawd standing, as a [`NavIcon::Image`]: the desktop pet's own picture.
pub fn clawd_mark(ctx: &Context) -> TextureId {
    nav_image(ctx, "clawd-mark", clawd_mask)
}

/// A [`NavIcon::Image`] from `mask`'s alpha for a square so many pixels across. It's drawn for
/// the window's scale, and again whenever that changes, so it's as sharp as the glyphs beside it.
fn nav_image(ctx: &Context, name: &'static str, mask: impl FnOnce(usize) -> Vec<u8>) -> TextureId {
    let pixels = nav_icon_pixels(ctx.pixels_per_point());
    let id = Id::new(name);
    let cached = ctx.data(|data| data.get_temp::<(usize, TextureHandle)>(id));
    if let Some((_, texture)) = cached.filter(|(size, _)| *size == pixels) {
        return texture.id();
    }
    let image = ColorImage::new([pixels; 2], mask(pixels).into_iter().map(Color32::from_white_alpha).collect());
    let texture = ctx.load_texture(name, image, TextureOptions::LINEAR);
    ctx.data_mut(|data| data.insert_temp(id, (pixels, texture.clone())));
    texture.id()
}

/// Clawd's sprite filling a square `pixels` across about as wide as the glyphs do, his eyes
/// cut out. Each cell of the sprite takes whole pixels, as nearest-neighbor scaling does, so
/// he stays crisp at any scale (and symmetric, as cells round outward from his middle alike).
fn clawd_mask(pixels: usize) -> Vec<u8> {
    let sprite = &mascot::STAND;
    let (columns, rows) = (sprite[0].len(), sprite.len());
    // A pixel either side at 100%, as the glyphs leave, and the same either side so he's centered.
    let left = (pixels as f32 / 32.0).round() as usize;
    let width = pixels - 2 * left;
    let height = (width as f32 * rows as f32 / columns as f32).round() as usize;
    let top = (pixels - height) / 2;
    // Where cell `i` of `cells` starts along a span.
    let edge = |i: usize, cells: usize, span: usize| (i as f32 * span as f32 / cells as f32).round() as usize;
    let mut mask = vec![0; pixels * pixels];
    for (row, line) in sprite.iter().enumerate() {
        for (column, _) in line.bytes().enumerate().filter(|&(_, cell)| cell == b'#') {
            let (x0, x1) = (left + edge(column, columns, width), left + edge(column + 1, columns, width));
            for y in top + edge(row, rows, height)..top + edge(row + 1, rows, height) {
                mask[y * pixels + x0..y * pixels + x1].fill(255);
            }
        }
    }
    mask
}

/// A note at the top of a page, like Windows' info bars.
pub fn info_bar(ui: &mut Ui, fonts: &Fonts, text: &str) {
    const MARK: f32 = 16.0;
    let width = ui.available_width();
    let wrap = (width - 2.0 * CARD_PADDING - MARK - 12.0).max(60.0);
    let galley = ui.painter().layout(text.to_owned(), FontId::proportional(BODY), PRIMARY, wrap);
    let height = (galley.size().y + 2.0 * CARD_PADDING_Y).max(48.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let painter = ui.painter();
    painter.rect(rect, 4, INSET, Stroke::new(1.0, SETTING_EDGE), StrokeKind::Inside);
    let first_line = rect.center().y - galley.size().y / 2.0 + galley.rows[0].height() / 2.0;
    let mark = pos2(rect.left() + CARD_PADDING + MARK / 2.0, first_line);
    painter.circle_filled(mark, MARK / 2.0, ACCENT);
    painter.text(mark + vec2(0.0, 0.5), Align2::CENTER_CENTER, "i", FontId::new(11.0, fonts.strong.clone()), ON_ACCENT);
    painter.galley(pos2(mark.x + MARK / 2.0 + 12.0, rect.center().y - galley.size().y / 2.0), galley, PRIMARY);
}

/// A setting on a card of its own, with its control on the right. `control_width` is the room
/// `add_control` takes. On a card too narrow for both, the control goes under the text.
pub fn setting(
    ui: &mut Ui,
    fonts: &Fonts,
    glyph: char,
    title: &str,
    description: Option<&str>,
    control_width: f32,
    add_control: impl FnOnce(&mut Ui),
) -> Response {
    let header = header(ui, fonts, glyph, title, description, control_width, Sense::hover());
    ui.painter().rect(header.rect, 4, SETTING, Stroke::new(1.0, SETTING_EDGE), StrokeKind::Inside);
    header.paint(ui.painter());
    let layout = Layout::right_to_left(Align::Center);
    let mut slot = ui.new_child(UiBuilder::new().max_rect(header.control).layout(layout));
    add_control(&mut slot);
    header.response
}

/// A card that opens a web page, like the links in Windows' Settings: an icon, a title and an
/// optional description, all of it clickable, with an arrow on the right saying it leads out
/// of the app.
pub fn link_card(ui: &mut Ui, fonts: &Fonts, icon: impl Into<NavIcon>, title: &str, description: Option<&str>) -> Response {
    let icon = icon.into();
    let glyph = match icon {
        NavIcon::Glyph(glyph) => glyph,
        NavIcon::Image(_) => ' ',
    };
    let mut header = header(ui, fonts, glyph, title, description, CHEVRON, Sense::click());
    // An image goes where the glyph would.
    if let NavIcon::Image(_) = icon {
        header.icon = None;
    }
    let response = &header.response;
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Link, ui.is_enabled(), title));
    let fill = if response.hovered() && !response.is_pointer_button_down_on() { SETTING_HOVER } else { SETTING };
    let painter = ui.painter();
    painter.rect(header.rect, 4, fill, Stroke::new(1.0, SETTING_EDGE), StrokeKind::Inside);
    header.paint(painter);
    if let NavIcon::Image(_) = icon {
        let center = pos2(header.rect.left() + CARD_PADDING + ICON_SIZE / 2.0, header.rect.center().y);
        paint_nav_icon(ui, fonts, icon, center, ICON_SIZE);
    }
    if let Some(font) = fonts.icon(14.0) {
        let arrow = pos2(header.control.right() - CHEVRON / 2.0, header.control.center().y);
        painter.text(arrow, Align2::CENTER_CENTER, icon::EXTERNAL, font, SECONDARY);
    }
    focus_ring(ui, response, header.rect, 4.0);
    header.response.on_hover_cursor(CursorIcon::PointingHand)
}

/// The card atop an About page, as in Windows' Settings: a big picture of the app beside its
/// name in large type, with `lines` under the name. `paint_picture` paints the picture into the
/// square it's given.
pub fn app_card(ui: &mut Ui, fonts: &Fonts, name: &str, lines: &[&str], paint_picture: impl FnOnce(&Painter, Rect)) {
    const PICTURE: f32 = 64.0;
    const PADDING: f32 = 20.0;
    let width = ui.available_width();
    let text_left = CARD_PADDING + PICTURE + PADDING;
    let wrap = (width - text_left - CARD_PADDING).max(60.0);
    let painter = ui.painter().clone();
    let name = painter.layout(name.to_owned(), FontId::new(20.0, fonts.strong.clone()), PRIMARY, wrap);
    let lines: Vec<_> =
        lines.iter().map(|line| painter.layout((*line).to_owned(), FontId::proportional(CAPTION), SECONDARY, wrap)).collect();
    let text_height = name.size().y + 2.0 + lines.iter().map(|line| line.size().y + 1.0).sum::<f32>();
    let (rect, _) = ui.allocate_exact_size(vec2(width, text_height.max(PICTURE) + 2.0 * PADDING), Sense::hover());
    painter.rect(rect, 4, SETTING, Stroke::new(1.0, SETTING_EDGE), StrokeKind::Inside);
    let picture = Rect::from_center_size(pos2(rect.left() + CARD_PADDING + PICTURE / 2.0, rect.center().y), Vec2::splat(PICTURE));
    paint_picture(&painter, picture);
    let mut at = pos2(rect.left() + text_left, rect.center().y - text_height / 2.0);
    let name_height = name.size().y;
    painter.galley(at, name, PRIMARY);
    at.y += name_height + 2.0;
    for line in lines {
        let height = line.size().y;
        painter.galley(at, line, SECONDARY);
        at.y += height + 1.0;
    }
}

/// A setting that opens onto more settings, like Windows' expanders: a card showing `summary`
/// and a chevron, which a click opens to show what `add_items` adds, such as [`check_item`]s.
pub fn expander(
    ui: &mut Ui,
    fonts: &Fonts,
    open: &mut bool,
    glyph: char,
    title: &str,
    description: Option<&str>,
    summary: &str,
    add_items: impl FnOnce(&mut Ui),
) {
    let id = ui.make_persistent_id(("expander", title));
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let width = ui.available_width();
        // The summary gets up to a third of the card, but never crowds the title under the
        // chevron, and is cut short past that. Too short to read, it's left out.
        let painter = ui.painter().clone();
        let text_room = width - CARD_PADDING - fonts.indent() - CARD_PADDING - CONTROL_GAP - NARROWEST_TEXT;
        let summary_room = (text_room - CHEVRON - 12.0).min(width / 3.0);
        let summary = (!summary.is_empty() && summary_room >= 40.0)
            .then(|| single_line(&painter, summary, FontId::proportional(BODY), SECONDARY, summary_room));
        let control_width = summary.as_ref().map_or(0.0, |s| s.size().x + 12.0) + CHEVRON;
        let background = painter.add(Shape::Noop);
        let mut header = header(ui, fonts, glyph, title, description, control_width, Sense::hover());
        header.response = ui.interact(header.rect, id, Sense::click());
        if header.response.clicked() {
            *open = !*open;
        }
        let open = *open;
        let response = &header.response;
        response.widget_info(|| WidgetInfo::selected(WidgetType::CollapsingHeader, ui.is_enabled(), open, title));
        let fill = if response.hovered() && !response.is_pointer_button_down_on() { SETTING_HOVER } else { SETTING };
        let corners = if open { CornerRadius { nw: 4, ne: 4, sw: 0, se: 0 } } else { CornerRadius::same(4) };
        let card = Shape::Rect(epaint_rect(header.rect, corners, fill, SETTING_EDGE));
        painter.set(background, card);
        header.paint(&painter);

        let control = header.control;
        let chevron = pos2(control.right() - 8.0, control.center().y);
        let glyph = if open { icon::CHEVRON_UP } else { icon::CHEVRON_DOWN };
        chevron_mark(&painter, fonts, chevron, glyph);
        if let Some(summary) = summary {
            let at = pos2(control.right() - CHEVRON - 12.0 - summary.size().x, control.center().y - summary.size().y / 2.0);
            painter.galley(at, summary, SECONDARY);
        }
        focus_ring(ui, response, header.rect, 4.0);
        let response = header.response.on_hover_cursor(CursorIcon::PointingHand);

        if open {
            under(ui, &painter, response.rect, add_items);
        }
    });
}

/// A card heading settings that always show under it, like an [`expander`] that stays open,
/// with `summary` on its right.
pub fn group<R>(
    ui: &mut Ui,
    fonts: &Fonts,
    glyph: char,
    title: &str,
    description: Option<&str>,
    summary: &str,
    add_items: impl FnOnce(&mut Ui) -> R,
) -> R {
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let painter = ui.painter().clone();
        let room = ui.available_width() / 3.0;
        let summary = single_line(&painter, summary, FontId::proportional(BODY), SECONDARY, room);
        let header = header(ui, fonts, glyph, title, description, summary.size().x, Sense::hover());
        let corners = CornerRadius { nw: 4, ne: 4, sw: 0, se: 0 };
        painter.add(Shape::Rect(epaint_rect(header.rect, corners, SETTING, SETTING_EDGE)));
        header.paint(&painter);
        let at = pos2(header.control.right() - summary.size().x, header.control.center().y - summary.size().y / 2.0);
        painter.galley(at, summary, SECONDARY);
        under(ui, &painter, header.rect, add_items)
    })
    .inner
}

/// Adds what `add_items` adds under a card's header, on a background down to the last of them.
fn under<R>(ui: &mut Ui, painter: &Painter, header: Rect, add_items: impl FnOnce(&mut Ui) -> R) -> R {
    let background = painter.add(Shape::Noop);
    let items = ui.vertical(add_items);
    // Up a pixel, so its edge and the header's are one line.
    let bottom = items.response.rect.bottom();
    let rect = Rect::from_min_max(pos2(header.left(), header.bottom() - 1.0), pos2(header.right(), bottom));
    let corners = CornerRadius { nw: 0, ne: 0, sw: 4, se: 4 };
    painter.set(background, Shape::Rect(epaint_rect(rect, corners, INSET, SETTING_EDGE)));
    items.inner
}

/// One of an [`expander`]'s settings: a check box beside a title and an optional description,
/// all of it clickable.
pub fn check_item(ui: &mut Ui, fonts: &Fonts, on: &mut bool, title: &str, description: Option<&str>) -> Response {
    let width = ui.available_width();
    let left = CARD_PADDING + fonts.indent();
    let text_left = left + CHECK_BOX + 12.0;
    let wrap = (width - text_left - CARD_PADDING).max(60.0);
    let painter = ui.painter().clone();
    let title_galley = painter.layout(title.to_owned(), FontId::proportional(BODY), PRIMARY, wrap);
    let description = description.map(|d| painter.layout(d.to_owned(), FontId::proportional(CAPTION), SECONDARY, wrap));
    let text_height = title_galley.size().y + description.as_ref().map_or(0.0, |d| 1.0 + d.size().y);
    let height = (text_height + 2.0 * CARD_PADDING_Y).max(48.0);

    let (rect, mut response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, ui.is_enabled(), *on, title));
    // A line along the top, over the last pixel row of what's above: the first one lies on the
    // header's edge, and the others part them.
    painter.hline(rect.x_range(), rect.top() - 0.5, Stroke::new(1.0, SETTING_EDGE));
    let mut at = pos2(rect.left() + text_left, rect.center().y - text_height / 2.0);
    let first_line = at.y + title_galley.rows[0].height() / 2.0;
    let check = Rect::from_center_size(pos2(rect.left() + left + CHECK_BOX / 2.0, first_line), vec2(CHECK_BOX, CHECK_BOX));
    paint_check_box(ui, fonts, check, *on, &response);
    let title_height = title_galley.size().y;
    painter.galley(at, title_galley, PRIMARY);
    if let Some(description) = description {
        at.y += title_height + 1.0;
        painter.galley(at, description, SECONDARY);
    }
    focus_ring(ui, &response, check, 4.0);
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// One of an [`expander`]'s rows that does something when clicked, like playing a sound: a
/// title and an optional description, with a button showing `glyph` on the right. All of it
/// is clickable.
pub fn action_item(ui: &mut Ui, fonts: &Fonts, glyph: char, title: &str, description: Option<&str>) -> Response {
    const BUTTON: f32 = 28.0;
    let width = ui.available_width();
    let text_left = CARD_PADDING + fonts.indent();
    let wrap = (width - text_left - CONTROL_GAP - BUTTON - CARD_PADDING).max(60.0);
    let painter = ui.painter().clone();
    let title_galley = painter.layout(title.to_owned(), FontId::proportional(BODY), PRIMARY, wrap);
    let description = description.map(|d| painter.layout(d.to_owned(), FontId::proportional(CAPTION), SECONDARY, wrap));
    let text_height = title_galley.size().y + description.as_ref().map_or(0.0, |d| 1.0 + d.size().y);
    let height = (text_height + 2.0 * CARD_PADDING_Y).max(48.0);

    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), title));
    // A line along the top, as between check items.
    painter.hline(rect.x_range(), rect.top() - 0.5, Stroke::new(1.0, SETTING_EDGE));
    let center = pos2(rect.right() - CARD_PADDING - BUTTON / 2.0, rect.center().y);
    let button = Rect::from_center_size(center, Vec2::splat(BUTTON));
    let fill = match (response.hovered(), response.is_pointer_button_down_on()) {
        (_, true) => SUBTLE_PRESSED,
        (true, false) => SUBTLE,
        (false, false) => Color32::TRANSPARENT,
    };
    painter.rect_filled(button, 4, fill);
    if let Some(font) = fonts.icon(14.0) {
        painter.text(button.center(), Align2::CENTER_CENTER, glyph, font, PRIMARY);
    }
    let mut at = pos2(rect.left() + text_left, rect.center().y - text_height / 2.0);
    let title_height = title_galley.size().y;
    painter.galley(at, title_galley, PRIMARY);
    if let Some(description) = description {
        at.y += title_height + 1.0;
        painter.galley(at, description, SECONDARY);
    }
    focus_ring(ui, &response, button, 4.0);
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// One of a [`group`]'s rows that only tells something: a title and an optional description,
/// with `value` on the right, like a number to read at a glance.
pub fn info_item(ui: &mut Ui, fonts: &Fonts, title: &str, description: Option<&str>, value: &str) -> Response {
    let width = ui.available_width();
    let text_left = CARD_PADDING + fonts.indent();
    let painter = ui.painter().clone();
    let value = painter.layout_no_wrap(value.to_owned(), FontId::new(BODY, fonts.strong.clone()), PRIMARY);
    let wrap = (width - text_left - CONTROL_GAP - value.size().x - CARD_PADDING).max(60.0);
    let title_galley = painter.layout(title.to_owned(), FontId::proportional(BODY), PRIMARY, wrap);
    let description = description.map(|d| painter.layout(d.to_owned(), FontId::proportional(CAPTION), SECONDARY, wrap));
    let text_height = title_galley.size().y + description.as_ref().map_or(0.0, |d| 1.0 + d.size().y);
    let height = (text_height + 2.0 * CARD_PADDING_Y).max(48.0);

    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, ui.is_enabled(), title));
    // A line along the top, as between check items.
    painter.hline(rect.x_range(), rect.top() - 0.5, Stroke::new(1.0, SETTING_EDGE));
    let mut at = pos2(rect.left() + text_left, rect.center().y - text_height / 2.0);
    let first_line = at.y + title_galley.rows[0].height() / 2.0;
    let value_at = pos2(rect.right() - CARD_PADDING - value.size().x, first_line - value.size().y / 2.0);
    painter.galley(value_at, value, PRIMARY);
    let title_height = title_galley.size().y;
    painter.galley(at, title_galley, PRIMARY);
    if let Some(description) = description {
        at.y += title_height + 1.0;
        painter.galley(at, description, SECONDARY);
    }
    response
}

/// A row of a [`sortable`] list.
pub struct SortRow<'a> {
    /// Tells the row apart from the others, whatever order they're in.
    pub id: Id,
    pub on: bool,
    pub title: &'a str,
    /// A line under the title, cut short when too long.
    pub description: &'a str,
}

/// What was done to a [`sortable`] list, for the caller to do to what it lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sorted {
    /// The row at this index was checked or unchecked.
    Toggled(usize),
    /// The row at `from` was moved to `to`, where it lands once out of the way.
    Moved { from: usize, to: usize },
}

/// Rows like [`check_item`]s that can also be dragged into another order, for under a
/// [`group`] or an [`expander`]. A gripper on the left shows where to grab, though a row drags
/// from anywhere. The dragged row lifts and follows the pointer, and the others slide out of
/// its way, leaving a slot where it will land. Escape puts it back, and Alt+Up and Alt+Down
/// move the row with keyboard focus. Returns what was done, if anything.
pub fn sortable(ui: &mut Ui, fonts: &Fonts, rows: &[SortRow]) -> Option<Sorted> {
    let width = ui.available_width();
    let (check_left, text_left) = sortable_columns(fonts);
    let room = (width - text_left - CARD_PADDING).max(60.0);
    let painter = ui.painter().clone();
    // A line of each, so the rows are all as tall and a dragged one fits any slot.
    let texts: Vec<_> = rows
        .iter()
        .map(|row| {
            let title = single_line(&painter, row.title, FontId::proportional(BODY), PRIMARY, room);
            let description = (!row.description.is_empty())
                .then(|| single_line(&painter, row.description, FontId::proportional(CAPTION), SECONDARY, room));
            (title, description)
        })
        .collect();
    let heights: Vec<f32> = texts
        .iter()
        .map(|(title, description)| {
            let text = title.size().y + description.as_ref().map_or(0.0, |d| 1.0 + d.size().y);
            (text + 2.0 * CARD_PADDING_Y).max(48.0)
        })
        .collect();
    // Laid out in the list's order. A drag only changes where they're painted.
    let responses: Vec<Response> = rows
        .iter()
        .zip(&heights)
        .map(|(row, &height)| {
            let (_, rect) = ui.allocate_space(vec2(width, height));
            ui.interact(rect, row.id, Sense::click_and_drag())
        })
        .collect();
    let list = responses.first()?.rect.union(responses.last()?.rect);
    let laid_out: Vec<f32> = responses.iter().map(|response| response.rect.top() - list.top()).collect();
    let pointer = ui.input(|i| i.pointer.latest_pos());
    let on_grip = |at: Pos2| at.x < list.left() + check_left - 6.0;

    let mut sorted = None;
    // The dragged row, how far down the list it shows, and where it would land.
    let mut drag = None;
    if let Some(from) = responses.iter().position(|r| r.dragged() || r.drag_stopped()) {
        let response = &responses[from];
        // Where on the row it was picked up, and whether Escape has put it back since.
        let held = rows[from].id.with("held");
        let half = heights[from] / 2.0;
        if response.drag_started() {
            let grab = ui.input(|i| i.pointer.press_origin()).map_or(half, |at| at.y - response.rect.top());
            ui.data_mut(|data| data.insert_temp(held, (grab, false)));
        }
        let (grab, mut put_back) = ui.data(|data| data.get_temp::<(f32, bool)>(held)).unwrap_or((half, false));
        if ui.input(|i| i.key_pressed(Key::Escape)) {
            put_back = true;
            ui.data_mut(|data| data.insert_temp(held, (grab, true)));
        }
        let pointer_y = pointer.map_or(response.rect.center().y, |at| at.y);
        let shown_at = (pointer_y - grab - list.top()).clamp(0.0, list.height() - heights[from]);
        // It lands past each row whose middle its own middle has passed.
        let middle = shown_at + half;
        let passed = (0..rows.len()).filter(|&i| i != from && laid_out[i] + heights[i] / 2.0 < middle).count();
        let to = if put_back { from } else { passed };
        if response.drag_stopped() {
            ui.data_mut(|data| data.remove_temp::<(f32, bool)>(held));
            if to != from {
                sorted = Some(Sorted::Moved { from, to });
            }
        } else {
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
            scroll_near_edge(ui, pointer_y);
        }
        drag = Some((from, shown_at, to));
    }

    // Where each row goes: in order, but with the dragged one where it would land.
    let mut order: Vec<usize> = (0..rows.len()).collect();
    if let Some((from, _, to)) = drag {
        order.remove(from);
        order.insert(to, from);
    }
    let mut slots = vec![0.0; rows.len()];
    let mut y = 0.0;
    for &i in &order {
        slots[i] = y;
        y += heights[i];
    }

    let row_rect = |y: f32, i: usize| Rect::from_min_size(pos2(list.left(), list.top() + y), vec2(width, heights[i]));
    for (i, (row, response)) in rows.iter().zip(&responses).enumerate() {
        response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, ui.is_enabled(), row.on, row.title));
        // A click on the gripper isn't one on the check box.
        if response.clicked() && !response.interact_pointer_pos().is_some_and(on_grip) {
            sorted = Some(Sorted::Toggled(i));
        }
        if response.has_focus() {
            let (up, down) = ui.input_mut(|input| {
                (input.consume_key(Modifiers::ALT, Key::ArrowUp), input.consume_key(Modifiers::ALT, Key::ArrowDown))
            });
            if up && i > 0 {
                sorted = Some(Sorted::Moved { from: i, to: i - 1 });
            } else if down && i + 1 < rows.len() {
                sorted = Some(Sorted::Moved { from: i, to: i + 1 });
            }
        }
        if drag.is_some_and(|(from, ..)| from == i) {
            continue;
        }
        let grip = drag.is_none() && response.hovered() && pointer.is_some_and(on_grip);
        if drag.is_none() && response.hovered() {
            ui.ctx().set_cursor_icon(if grip { CursorIcon::Grab } else { CursorIcon::PointingHand });
        }
        let y = ui.ctx().animate_value_with_time(row.id.with("at"), slots[i], SLIDE);
        let rect = row_rect(y, i);
        // A line along the top: the first one lies on the header's edge, and the others part them.
        ui.painter().hline(rect.x_range(), rect.top() - 0.5, Stroke::new(1.0, SETTING_EDGE));
        paint_sortable_row(ui, fonts, rect, &texts[i], row.on, response, grip);
    }
    if let Some((from, shown_at, _)) = drag {
        // The slot it will land in, then the row itself, lifted over the others.
        let slot = row_rect(slots[from], from).shrink(3.0);
        ui.painter().rect(slot, 4, WINDOW, Stroke::new(1.0, CONTROL_EDGE), StrokeKind::Inside);
        let rect = row_rect(shown_at, from);
        let shadow = Shadow { offset: [0, 6], blur: 16, spread: 0, color: Color32::from_black_alpha(110) };
        ui.painter().add(shadow.as_shape(rect, 4));
        ui.painter().rect(rect, 4, SETTING_HOVER, Stroke::new(1.0, CONTROL_EDGE), StrokeKind::Inside);
        paint_sortable_row(ui, fonts, rect, &texts[from], rows[from].on, &responses[from], true);
        // Kept up with, so once let go it glides from here to its slot.
        ui.ctx().animate_value_with_time(rows[from].id.with("at"), shown_at, 0.0);
    }
    sorted
}

/// Where a [`sortable`] row's check box and text start. Its gripper takes the icon column of
/// the card above, or a lane of its own without the icon font.
fn sortable_columns(fonts: &Fonts) -> (f32, f32) {
    let check_left = CARD_PADDING + if fonts.icons.is_some() { fonts.indent() } else { GRIP_LANE };
    (check_left, check_left + CHECK_BOX + 12.0)
}

/// Scrolls the list a row is dragged in while the pointer is near the top or bottom of what
/// shows of it.
fn scroll_near_edge(ui: &Ui, pointer_y: f32) {
    let shown = ui.clip_rect();
    let speed = if pointer_y < shown.top() + EDGE {
        EDGE_SCROLL
    } else if pointer_y > shown.bottom() - EDGE {
        -EDGE_SCROLL
    } else {
        return;
    };
    let dt = ui.input(|i| i.stable_dt).min(0.1);
    ui.scroll_with_delta_animation(vec2(0.0, speed * dt), ScrollAnimation::none());
    ui.ctx().request_repaint();
}

/// A [`sortable`] row's gripper, lit while it's grabbed or about to be, check box, title and
/// description.
fn paint_sortable_row(
    ui: &Ui,
    fonts: &Fonts,
    rect: Rect,
    (title, description): &(Arc<Galley>, Option<Arc<Galley>>),
    on: bool,
    response: &Response,
    grip_lit: bool,
) {
    let painter = ui.painter();
    let (check_left, text_left) = sortable_columns(fonts);
    let grip_color = if grip_lit { PRIMARY } else { SECONDARY };
    if let Some(font) = fonts.icon(16.0) {
        let grip = pos2(rect.left() + CARD_PADDING + ICON_SIZE / 2.0, rect.center().y);
        painter.text(grip, Align2::CENTER_CENTER, icon::REORDER, font, grip_color);
    } else {
        let grip = rect.left() + CARD_PADDING + GRIP_LANE / 2.0 - 6.0;
        for dy in [-2.5, 2.5] {
            painter.hline(grip - 6.0..=grip + 6.0, rect.center().y + dy, Stroke::new(1.0, grip_color));
        }
    }
    let text_height = title.size().y + description.as_ref().map_or(0.0, |d| 1.0 + d.size().y);
    let mut at = pos2(rect.left() + text_left, rect.center().y - text_height / 2.0);
    let first_line = at.y + title.rows[0].height() / 2.0;
    let check = Rect::from_center_size(pos2(rect.left() + check_left + CHECK_BOX / 2.0, first_line), vec2(CHECK_BOX, CHECK_BOX));
    paint_check_box(ui, fonts, check, on, response);
    painter.galley(at, title.clone(), PRIMARY);
    if let Some(description) = description {
        at.y += title.size().y + 1.0;
        painter.galley(at, description.clone(), SECONDARY);
    }
    focus_ring(ui, response, check, 4.0);
}

/// A card's icon, title and description, laid out but not yet painted.
struct Header {
    rect: Rect,
    response: Response,
    /// Where the control goes.
    control: Rect,
    icon: Option<(Arc<Galley>, emath::Pos2)>,
    text: Vec<(Arc<Galley>, emath::Pos2, Color32)>,
}

impl Header {
    fn paint(&self, painter: &Painter) {
        if let Some((icon, at)) = &self.icon {
            painter.galley(*at, icon.clone(), PRIMARY);
        }
        for (galley, at, color) in &self.text {
            painter.galley(*at, galley.clone(), *color);
        }
    }
}

/// Allocates a card for a setting, with its text beside `control_width` or, when the card is
/// too narrow for that, above it.
fn header(
    ui: &mut Ui,
    fonts: &Fonts,
    glyph: char,
    title: &str,
    description: Option<&str>,
    control_width: f32,
    sense: Sense,
) -> Header {
    let width = ui.available_width();
    let text_left = CARD_PADDING + fonts.indent();
    let beside = width - text_left - CARD_PADDING - CONTROL_GAP - control_width;
    let stacked = control_width > 0.0 && beside < NARROWEST_TEXT;
    let wrap = if stacked || control_width <= 0.0 { width - text_left - CARD_PADDING } else { beside }.max(60.0);
    let painter = ui.painter().clone();
    let title = painter.layout(title.to_owned(), FontId::proportional(BODY), PRIMARY, wrap);
    let description = description.map(|d| painter.layout(d.to_owned(), FontId::proportional(CAPTION), SECONDARY, wrap));
    let text_height = title.size().y + description.as_ref().map_or(0.0, |d| 1.0 + d.size().y);
    let height = if stacked {
        CARD_PADDING_Y + text_height + 8.0 + CONTROL_HEIGHT + CARD_PADDING_Y
    } else {
        (text_height + 2.0 * CARD_PADDING_Y).max(CARD_HEIGHT)
    };

    let (rect, response) = ui.allocate_exact_size(vec2(width, height), sense);
    let text_top = if stacked { rect.top() + CARD_PADDING_Y } else { rect.center().y - text_height / 2.0 };
    let control = if stacked {
        let top = text_top + text_height + 8.0;
        let width = control_width.min(width - text_left - CARD_PADDING);
        Rect::from_min_size(pos2(rect.left() + text_left, top), vec2(width, CONTROL_HEIGHT))
    } else {
        Rect::from_min_max(pos2(rect.right() - CARD_PADDING - control_width, rect.top()), pos2(rect.right() - CARD_PADDING, rect.bottom()))
    };
    let icon = fonts.icon(ICON_SIZE).map(|font| {
        let galley = painter.layout_no_wrap(glyph.to_string(), font, PRIMARY);
        let center_y = if stacked { text_top + title.size().y / 2.0 } else { rect.center().y };
        let at = pos2(rect.left() + CARD_PADDING + ICON_SIZE / 2.0, center_y) - galley.size() / 2.0;
        (galley, at)
    });
    let mut at = pos2(rect.left() + text_left, text_top);
    let mut text = vec![];
    let title_height = title.size().y;
    text.push((title, at, PRIMARY));
    if let Some(description) = description {
        at.y += title_height + 1.0;
        text.push((description, at, SECONDARY));
    }
    Header { rect, response, control, icon, text }
}

fn epaint_rect(rect: Rect, corners: CornerRadius, fill: Color32, edge: Color32) -> eframe::egui::epaint::RectShape {
    eframe::egui::epaint::RectShape::new(rect, corners, fill, Stroke::new(1.0, edge), StrokeKind::Inside)
}

/// A chevron centered on `at`, from the icon font or drawn.
fn chevron_mark(painter: &Painter, fonts: &Fonts, at: emath::Pos2, glyph: char) {
    if let Some(font) = fonts.icon(11.0) {
        painter.text(at, Align2::CENTER_CENTER, glyph, font, SECONDARY);
    } else {
        let flip = if glyph == icon::CHEVRON_UP { -1.0 } else { 1.0 };
        let points = vec![at + vec2(-4.0, -2.0 * flip), at + vec2(0.0, 2.0 * flip), at + vec2(4.0, -2.0 * flip)];
        painter.add(Shape::line(points, Stroke::new(1.2, SECONDARY)));
    }
}

/// An on/off switch with On or Off beside it, as in Windows' Settings.
pub fn toggle(ui: &mut Ui, on: &mut bool) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(vec2(TOGGLE_WIDTH, CONTROL_HEIGHT), Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, ui.is_enabled(), *on, ""));

    let t = ui.ctx().animate_bool_responsive(response.id, *on);
    let (hovered, pressed) = (response.hovered(), response.is_pointer_button_down_on());
    // The track gets snapped to whole pixels when painted but the knob doesn't, so snap the
    // track here and center the knob in it. Otherwise the knob can sit half a pixel high.
    let switch = Rect::from_min_size(pos2(rect.right() - SWITCH[0], rect.center().y - SWITCH[1] / 2.0), SWITCH.into())
        .round_to_pixels(ui.pixels_per_point());
    let radius = switch.height() / 2.0;
    let painter = ui.painter();
    let off = if hovered { OFF_HOVER } else { OFF };
    let accent = if hovered { ACCENT.lerp_to_gamma(SETTING, 0.12) } else { ACCENT };
    painter.rect_filled(switch, radius, off.lerp_to_gamma(accent, t));
    if t < 1.0 {
        let edge = STRONG.gamma_multiply(1.0 - t);
        painter.rect_stroke(switch, radius, Stroke::new(1.0, edge), StrokeKind::Inside);
    }
    let knob = if pressed { 7.5 } else if hovered { 7.0 } else { 6.0 };
    let x = emath::lerp(switch.left() + radius..=switch.right() - radius, t);
    painter.circle_filled(pos2(x, switch.center().y), knob, SECONDARY.lerp_to_gamma(ON_ACCENT, t));
    let state = if *on { "On" } else { "Off" };
    let label = pos2(switch.left() - 10.0, rect.center().y);
    painter.text(label, Align2::RIGHT_CENTER, state, FontId::proportional(BODY), PRIMARY);
    focus_ring(ui, &response, switch, radius);
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// A check box, filled with the accent color and checked while on.
fn paint_check_box(ui: &Ui, fonts: &Fonts, rect: Rect, on: bool, response: &Response) {
    let rect = rect.round_to_pixels(ui.pixels_per_point());
    let hovered = response.hovered();
    let painter = ui.painter();
    if on {
        let fill = if hovered { ACCENT.lerp_to_gamma(SETTING, 0.12) } else { ACCENT };
        painter.rect_filled(rect, 4, fill);
        if let Some(font) = fonts.icon(12.0) {
            painter.text(rect.center(), Align2::CENTER_CENTER, icon::CHECK, font, ON_ACCENT);
        } else {
            let c = rect.center();
            let points = vec![c + vec2(-5.0, 0.0), c + vec2(-1.5, 3.5), c + vec2(5.0, -3.5)];
            painter.add(Shape::line(points, Stroke::new(1.5, ON_ACCENT)));
        }
    } else {
        painter.rect(rect, 4, if hovered { OFF_HOVER } else { OFF }, Stroke::new(1.0, STRONG), StrokeKind::Inside);
    }
}

/// The width [`dropdown`] wants for these labels.
pub fn dropdown_width(ui: &Ui, labels: &[&str]) -> f32 {
    let widest = labels.iter().map(|l| control_text(ui, l).size().x).fold(0.0, f32::max);
    (12.0 + widest + CHEVRON_ROOM).max(DROPDOWN_WIDTH)
}

/// Picks one of `options` from a list that drops down when clicked, like Windows' combo boxes.
/// `width` is the most it takes.
pub fn dropdown<T: Copy + PartialEq>(
    ui: &mut Ui,
    fonts: &Fonts,
    value: &mut T,
    options: &[(T, &str)],
    width: f32,
) -> Response {
    let width = width.min(ui.available_width());
    let (rect, mut response) = ui.allocate_exact_size(vec2(width, CONTROL_HEIGHT), Sense::click());
    let popup = response.id.with("popup");
    let open = Popup::is_id_open(ui.ctx(), popup);
    let selected = options.iter().find(|(option, _)| *option == *value).map_or("", |(_, label)| *label);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::ComboBox, ui.is_enabled(), selected));

    let fill = match (response.hovered(), response.is_pointer_button_down_on() || open) {
        (_, true) => CONTROL_PRESSED,
        (true, false) => CONTROL_HOVER,
        _ => CONTROL,
    };
    let painter = ui.painter();
    painter.rect(rect, 4, fill, Stroke::new(1.0, CONTROL_EDGE), StrokeKind::Inside);
    let text = single_line(painter, selected, FontId::proportional(BODY), PRIMARY, width - 12.0 - CHEVRON_ROOM);
    painter.galley(pos2(rect.left() + 12.0, rect.center().y - text.size().y / 2.0), text, PRIMARY);
    chevron_mark(painter, fonts, pos2(rect.right() - 17.0, rect.center().y), icon::CHEVRON_DOWN);
    focus_ring(ui, &response, rect, 4.0);

    let mut changed = false;
    Popup::menu(&response).id(popup).width(width).gap(4.0).frame(flyout_frame()).show(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.set_min_width(ui.available_width());
        for (option, label) in options {
            let (item, clicked) = ui.allocate_exact_size(vec2(ui.available_width(), CONTROL_HEIGHT), Sense::click());
            let picked = *option == *value;
            clicked.widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, true, picked, *label));
            let fill = match (clicked.hovered(), clicked.is_pointer_button_down_on()) {
                (_, true) => SUBTLE_PRESSED,
                (true, false) => CONTROL_HOVER,
                _ if picked => CONTROL,
                _ => Color32::TRANSPARENT,
            };
            let painter = ui.painter();
            painter.rect_filled(item, 4, fill);
            if picked {
                let bar = Rect::from_center_size(pos2(item.left() + 1.5, item.center().y), vec2(3.0, 16.0));
                painter.rect_filled(bar, 1.5, ACCENT);
            }
            let text = single_line(painter, label, FontId::proportional(BODY), PRIMARY, item.width() - 24.0);
            painter.galley(pos2(item.left() + 12.0, item.center().y - text.size().y / 2.0), text, PRIMARY);
            if clicked.clicked() && !picked {
                *value = *option;
                changed = true;
            }
        }
    });
    if changed {
        response.mark_changed();
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// What a flyout shows on, such as a [`dropdown`]'s list or a menu: a card lifted off the
/// window by a shadow, as Windows' flyouts are.
pub fn flyout_frame() -> Frame {
    Frame::new()
        .fill(POPUP)
        .stroke(Stroke::new(1.0, POPUP_EDGE))
        .corner_radius(8)
        .inner_margin(Margin::same(4))
        .shadow(Shadow { offset: [0, 8], blur: 16, spread: 0, color: Color32::from_black_alpha(100) })
}

/// One of a flyout menu's commands: an icon and a label across the menu, lit under the pointer.
pub fn menu_item(ui: &mut Ui, fonts: &Fonts, glyph: char, label: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), CONTROL_HEIGHT), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), label));
    let fill = match (response.hovered(), response.is_pointer_button_down_on()) {
        (_, true) => SUBTLE_PRESSED,
        (true, false) => CONTROL_HOVER,
        (false, false) => Color32::TRANSPARENT,
    };
    let painter = ui.painter();
    painter.rect_filled(rect, 4, fill);
    let mut x = rect.left() + 12.0;
    if let Some(font) = fonts.icon(14.0) {
        painter.text(pos2(x + 7.0, rect.center().y), Align2::CENTER_CENTER, glyph, font, PRIMARY);
        x += 14.0 + 12.0;
    }
    painter.text(pos2(x, rect.center().y), Align2::LEFT_CENTER, label, FontId::proportional(BODY), PRIMARY);
    focus_ring(ui, &response, rect, 4.0);
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// A line across a flyout menu between its parts.
pub fn menu_separator(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 9.0), Sense::hover());
    ui.painter().hline(rect.x_range(), rect.center().y, Stroke::new(1.0, POPUP_EDGE));
}

/// A horizontal slider. Arrow keys nudge it when focused.
pub fn slider(ui: &mut Ui, value: &mut f32, range: RangeInclusive<f32>, width: f32) -> Response {
    let mut t = emath::remap_clamp(*value, range.clone(), 0.0..=1.0);
    let response = slide(ui, &mut t, None, width);
    if response.changed() {
        *value = emath::lerp(range, t);
    }
    response.widget_info(|| WidgetInfo::slider(ui.is_enabled(), f64::from(*value), ""));
    response
}

/// A slider that stops only at `count` steps, each ticked under its track. `step` is the one
/// it's on, counting from 0. Arrow keys move it a step when focused.
pub fn stepped_slider(ui: &mut Ui, step: &mut usize, count: usize, width: f32) -> Response {
    let last = count.max(2) - 1;
    let mut t = (*step).min(last) as f32 / last as f32;
    let response = slide(ui, &mut t, Some(last), width);
    *step = (t * last as f32).round() as usize;
    response.widget_info(|| WidgetInfo::slider(ui.is_enabled(), *step as f64 + 1.0, ""));
    response
}

/// What both sliders share, at `t` from 0 to 1. With `steps`, `t` stops only at that many even
/// divisions of the track, each ticked, and an arrow key moves it one; otherwise a twentieth.
/// Drawn like Windows' sliders: a thin track, filled up to a round thumb with an accent center.
fn slide(ui: &mut Ui, t: &mut f32, steps: Option<usize>, width: f32) -> Response {
    const THUMB_RADIUS: f32 = 10.0;
    let (rect, mut response) = ui.allocate_exact_size(vec2(width, CONTROL_HEIGHT), Sense::click_and_drag());
    let track = rect.shrink2(vec2(THUMB_RADIUS, 0.0));
    let nudge = steps.map_or(0.05, |steps| 1.0 / steps as f32);
    let snap = |t: f32| steps.map_or(t, |steps| (t * steps as f32).round() / steps as f32);
    let before = *t;
    if let Some(pointer) = response.interact_pointer_pos() {
        *t = snap(((pointer.x - track.left()) / track.width()).clamp(0.0, 1.0));
    }
    if response.has_focus() {
        let filter = EventFilter { horizontal_arrows: true, ..Default::default() };
        ui.memory_mut(|memory| memory.set_focus_lock_filter(response.id, filter));
        let presses = ui.input(|i| i.num_presses(Key::ArrowRight) as f32 - i.num_presses(Key::ArrowLeft) as f32);
        *t = snap((*t + presses * nudge).clamp(0.0, 1.0));
    }
    if *t != before {
        response.mark_changed();
    }

    let x = emath::lerp(track.x_range(), *t);
    let y = rect.center().y;
    let bar = |from: f32, to: f32| Rect::from_min_max(pos2(from, y - 2.0), pos2(to, y + 2.0));
    let painter = ui.painter();
    if let Some(steps) = steps {
        for i in 0..=steps {
            let at = emath::lerp(track.x_range(), i as f32 / steps as f32).round() + 0.5;
            painter.vline(at, y + 6.0..=y + 10.0, Stroke::new(1.0, STRONG));
        }
    }
    painter.rect_filled(bar(track.left(), track.right()), 2, STRONG);
    painter.rect_filled(bar(track.left(), x), 2, ACCENT);
    let thumb = pos2(x, y);
    painter.circle(thumb, THUMB_RADIUS, THUMB, Stroke::new(1.0, CONTROL_EDGE));
    let center = if response.dragged() || response.is_pointer_button_down_on() {
        5.0
    } else if response.hovered() {
        7.0
    } else {
        6.0
    };
    painter.circle_filled(thumb, center, ACCENT);
    focus_ring(ui, &response, Rect::from_center_size(thumb, vec2(2.0 * THUMB_RADIUS, 2.0 * THUMB_RADIUS)), THUMB_RADIUS);
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// A borderless square button showing an icon, or a plain button saying `label` without the
/// icon font. `label` also names it for screen readers.
pub fn icon_button(ui: &mut Ui, fonts: &Fonts, glyph: char, label: &str) -> Response {
    let Some(font) = fonts.icon(14.0) else { return ui.button(label) };
    let (rect, response) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), label));
    let hovered = response.hovered();
    let painter = ui.painter();
    if hovered {
        painter.rect_filled(rect, 4, if response.is_pointer_button_down_on() { SUBTLE_PRESSED } else { SUBTLE });
    }
    painter.text(rect.center(), Align2::CENTER_CENTER, glyph, font, if hovered { PRIMARY } else { SECONDARY });
    focus_ring(ui, &response, rect, 4.0);
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// The width [`button`] takes for this label.
pub fn button_width(ui: &Ui, fonts: &Fonts, label: &str) -> f32 {
    let icon_width = if fonts.icons.is_some() { 14.0 + 8.0 } else { 0.0 };
    12.0 + icon_width + control_text(ui, label).size().x + 12.0
}

/// A button like Windows' standard ones, with an icon before its label.
pub fn button(ui: &mut Ui, fonts: &Fonts, glyph: char, label: &str) -> Response {
    any_button(ui, fonts, glyph, label, false)
}

/// A [`button`] in the accent color, for what a page is there to do, like Windows Update's
/// Download and install.
pub fn accent_button(ui: &mut Ui, fonts: &Fonts, glyph: char, label: &str) -> Response {
    any_button(ui, fonts, glyph, label, true)
}

fn any_button(ui: &mut Ui, fonts: &Fonts, glyph: char, label: &str, accent: bool) -> Response {
    let text = control_text(ui, label);
    let icon = fonts.icon(14.0).map(|font| ui.painter().layout_no_wrap(glyph.into(), font, PRIMARY));
    let icon_width = icon.as_ref().map_or(0.0, |_| 14.0 + 8.0);
    let size = vec2(12.0 + icon_width + text.size().x + 12.0, CONTROL_HEIGHT);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), label));
    let pressed = response.is_pointer_button_down_on();
    let (fill, edge) = match (accent, response.hovered(), pressed) {
        (true, _, true) => (ACCENT.gamma_multiply(0.8), ACCENT),
        (true, true, false) => (ACCENT.gamma_multiply(0.9), ACCENT),
        (true, false, false) => (ACCENT, ACCENT),
        (false, _, true) => (CONTROL_PRESSED, CONTROL_EDGE),
        (false, true, false) => (CONTROL_HOVER, CONTROL_EDGE),
        (false, false, false) => (CONTROL, CONTROL_EDGE),
    };
    let color = match (accent, pressed) {
        (true, _) => ON_ACCENT,
        (false, true) => SECONDARY,
        (false, false) => PRIMARY,
    };
    let painter = ui.painter();
    painter.rect(rect, 4, fill, Stroke::new(1.0, edge), StrokeKind::Inside);
    let mut x = rect.left() + 12.0;
    if let Some(icon) = icon {
        painter.galley(pos2(x, rect.center().y - icon.size().y / 2.0), icon, color);
        x += icon_width;
    }
    painter.galley(pos2(x, rect.center().y - text.size().y / 2.0), text, color);
    focus_ring(ui, &response, rect, 4.0);
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// `text` on one line, cut short with an ellipsis past `width`.
pub fn single_line(painter: &Painter, text: &str, font: FontId, color: Color32, width: f32) -> Arc<Galley> {
    let mut job = LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = TextWrapping::truncate_at_width(width);
    painter.layout_job(job)
}

/// Text for a control, colored when painted.
fn control_text(ui: &Ui, text: &str) -> Arc<Galley> {
    ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(BODY), Color32::PLACEHOLDER)
}

/// Shows which control has keyboard focus, with Windows' white and black rings.
fn focus_ring(ui: &Ui, response: &Response, rect: Rect, radius: f32) {
    if response.has_focus() {
        let painter = ui.painter();
        painter.rect_stroke(rect.expand(1.0), radius + 1.0, Stroke::new(1.0, Color32::BLACK), StrokeKind::Outside);
        painter.rect_stroke(rect.expand(2.0), radius + 2.0, Stroke::new(2.0, Color32::WHITE), StrokeKind::Outside);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_camera_is_a_rounded_square_and_a_ring_with_a_flash() {
        let mask = vector::mask(CAMERA_MARK, 16.0, 32);
        let at = |x: usize, y: usize| mask[y * 32 + x];
        // Its frame, the lens's ring, and the flash.
        for (x, y) in [(16, 2), (16, 10), (24, 8)] {
            assert_eq!(at(x, y), 255, "nothing at {x}, {y}");
        }
        // Outside the corner, inside the frame, and inside the lens.
        for (x, y) in [(0, 0), (16, 6), (16, 16)] {
            assert_eq!(at(x, y), 0, "something at {x}, {y}");
        }
    }

    #[test]
    fn clawd_mark_is_symmetric_and_as_wide_as_the_glyphs() {
        // From 100% to 300% scaling.
        for pixels in 16..=48 {
            let mask = clawd_mask(pixels);
            assert_eq!(mask.len(), pixels * pixels);
            for row in mask.chunks(pixels) {
                assert!(row.iter().eq(row.iter().rev()), "lopsided at {pixels} pixels");
            }
            let lit = |x: usize| mask.chunks(pixels).any(|row| row[x] > 0);
            let width = (0..pixels).filter(|&x| lit(x)).count();
            assert!(width + 4 >= pixels && width < pixels, "{width} of {pixels} pixels wide");
        }
    }
}
