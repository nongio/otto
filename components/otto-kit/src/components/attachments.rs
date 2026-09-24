//! A list of attachments: things handed to Ask to ask about.
//!
//! Selected text reads as itself; a screen region is its picture; a picture
//! file is its picture under its name; any other file is its icon and name,
//! as in Files. The same list is the otto-gather card, the files attached to
//! a request in Ask, and the files that went with one in its log.
//!
//! [`AttachmentList`] lays a list out and paints it. It keeps what it has
//! read about each file (its picture, icon, kind and size), so laying out
//! again is cheap. The layout is in its own coordinates: `x` from `0` to the
//! width asked for, `y` from `0` down. An item's hover highlight reaches
//! [`HOVER_PAD`] past that on every side, so a caller leaves room for it.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use skia_safe::canvas::SrcRectConstraint;
use skia_safe::font_style::{Slant, Weight, Width};
use skia_safe::textlayout::{
    FontCollection, Paragraph, ParagraphBuilder, ParagraphStyle, TextDecoration, TextStyle,
};
use skia_safe::{
    surfaces, Canvas, ClipOp, Color, CubicResampler, Data, FontMgr, FontStyle, Image, Paint,
    PaintStyle, Point, RRect, Rect, SamplingOptions,
};

use crate::components::text_input::TextInputStyle;
use crate::filetype::{self, Kind};
use crate::icons;
use crate::theme::Theme;
use crate::typography::styles;

/// How far an item's hover highlight reaches past it, in points.
pub const HOVER_PAD: f32 = 6.0;
const HOVER_RADIUS: f32 = 10.0;
/// Between items.
const ROW_GAP: f32 = 14.0;
const PICTURE_RADIUS: f32 = 8.0;
const CROSS: f32 = 12.0;
/// Around a remove button's cross, what the pointer can hit.
const CROSS_SLOP: f32 = 6.0;
/// The dark disc a remove button sits on over a picture.
const DISC: f32 = 22.0;
/// How much of a struck item shows.
const STRUCK_ALPHA: f32 = 0.4;
/// The tallest a screen region's picture is shown.
const PICTURE_MAX_H: f32 = 220.0;
/// The shortest a picture is shown, so a thin strip is still a picture.
const PICTURE_MIN_H: f32 = 60.0;
/// A file's icon or thumbnail, as in a Files list row with two lines.
pub const ICON_SIZE: f32 = 48.0;
/// A selection's text: its size, line height and most lines shown.
const TEXT_SIZE: f32 = 13.0;
const TEXT_LINE_H: f32 = 19.0;
const TEXT_LINES: usize = 3;
/// How much of a file is read to tell what it is.
const PEEK_BYTES: u64 = 4096;
/// Pictures larger than this aren't decoded for a preview.
const PICTURE_MAX_BYTES: u64 = 40 << 20;
/// Previews are kept at this many pixels per point.
const PREVIEW_SCALE: f32 = 2.0;
/// The widest a list is expected to be, for sizing previews.
const PREVIEW_MAX_W: f32 = 480.0;

/// Where otto-gather writes what it gathers, under the runtime directory.
pub const GATHER_DIR: &str = "otto-gather";

/// One thing to ask about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Attachment {
    /// Text selected in an app.
    Text(String),
    /// A file.
    File(PathBuf),
    /// A captured screen region, saved as a PNG.
    Region(PathBuf),
}

impl Attachment {
    /// The attachment a file handed over stands for.
    ///
    /// otto-gather hands text and screen regions over as files in
    /// [`GATHER_DIR`]: `selection-N.txt` and `region-N.png`. Those read as
    /// the text and the region they are; any other path is a file.
    pub fn for_file(path: &Path) -> Self {
        match gather_dir() {
            Some(dir) => Self::for_file_in(path, &dir),
            None => Self::File(path.to_owned()),
        }
    }

    /// [`Self::for_file`], with otto-gather's directory at `gather_dir`.
    fn for_file_in(path: &Path, gather_dir: &Path) -> Self {
        let gathered = path.starts_with(gather_dir);
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if gathered && name.starts_with("selection-") && name.ends_with(".txt") {
            if let Ok(text) = std::fs::read_to_string(path) {
                return Self::Text(text);
            }
        }
        if gathered && name.starts_with("region-") && name.ends_with(".png") {
            return Self::Region(path.to_owned());
        }
        Self::File(path.to_owned())
    }

    /// The file behind it, if it is one.
    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::Text(_) => None,
            Self::File(path) | Self::Region(path) => Some(path),
        }
    }
}

/// otto-gather's directory: [`GATHER_DIR`] in the runtime directory.
pub fn gather_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR").map(|runtime| PathBuf::from(runtime).join(GATHER_DIR))
}

/// How a list is laid out.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// How wide the list is, in points.
    pub width: f32,
    /// Newest first, as a pile being gathered; otherwise in the order
    /// given, as a record of what went.
    pub newest_first: bool,
    /// Give each item a remove button.
    pub removable: bool,
}

/// What is known about a file, read once while it is listed.
struct Preview {
    /// A screen region's picture, scaled down to the widest a list is.
    picture: Option<Image>,
    bytes: Option<u64>,
    /// The file's icon from the icon theme, as Files shows it.
    icon: Option<Image>,
    /// What kind of file it is, in words: "Text", "Folder".
    kind: Option<&'static str>,
    /// The file's thumbnail, shown in place of its icon as Files does.
    thumbnail: Thumbnail,
}

/// Where a file's thumbnail is up to. The list doesn't make thumbnails: the
/// app asks for them ([`AttachmentList::thumbnails_wanted`]) and hands them
/// in ([`AttachmentList::set_thumbnail`]), as Files does, decoding untrusted
/// files in its sandbox.
#[derive(Clone)]
enum Thumbnail {
    /// Not asked for yet.
    Unknown,
    /// Asked for, not yet handed in.
    Asked,
    Found(Image),
    /// There is none: the file is not a picture, or could not be read.
    None,
}

enum Piece {
    Text(Paragraph, Point),
    /// A text selection's highlight, one rectangle per line.
    Selection {
        rects: Vec<Rect>,
        color: Color,
    },
    Box {
        rect: Rect,
        radius: f32,
        fill: Color,
        stroke: Option<Color>,
    },
    /// `image` covering `rect`, cropped to its shape.
    Picture {
        image: Image,
        rect: Rect,
        radius: f32,
    },
    /// An icon, drawn whole in `rect`.
    Icon {
        image: Image,
        rect: Rect,
    },
    /// A picture in an icon's place, as Files shows one: fitted into `rect`
    /// and sat on its bottom edge, never enlarged, edged with a hairline.
    Thumbnail {
        image: Image,
        rect: Rect,
    },
    /// A remove button's cross, in this colour.
    Cross(Point, Color),
    /// Pieces drawn as they are.
    Group(Vec<Piece>),
    /// Pieces drawn see-through, for a struck item.
    Faded(Vec<Piece>),
    /// An item on its way out: shrunk about `center` by `presence`, and as
    /// see-through, moved up by `lift` to where its closing slot is.
    Leaving {
        pieces: Vec<Piece>,
        center: Point,
        presence: f32,
        lift: f32,
    },
}

/// A list laid out, ready to paint.
pub struct Layout {
    pieces: Vec<Piece>,
    /// The remove buttons, by item index.
    removes: Vec<(Rect, usize)>,
    /// Where each item is, highlight included, by its index.
    items_at: Vec<(Rect, usize)>,
    /// How tall the list is, in points.
    pub height: f32,
}

fn find(rects: &[(Rect, usize)], x: f32, y: f32) -> Option<usize> {
    rects
        .iter()
        .find(|(rect, _)| {
            (rect.left..rect.right).contains(&x) && (rect.top..rect.bottom).contains(&y)
        })
        .map(|&(_, index)| index)
}

impl Layout {
    /// The item whose remove button is at `(x, y)`.
    pub fn remove_at(&self, x: f32, y: f32) -> Option<usize> {
        find(&self.removes, x, y)
    }

    /// The item at `(x, y)`.
    pub fn item_at(&self, x: f32, y: f32) -> Option<usize> {
        find(&self.items_at, x, y)
    }
}

/// Lays attachments out and paints them, remembering what it read about
/// each file.
pub struct AttachmentList {
    fonts: FontCollection,
    previews: HashMap<PathBuf, Preview>,
}

impl Default for AttachmentList {
    fn default() -> Self {
        let mut fonts = FontCollection::new();
        fonts.set_default_font_manager(FontMgr::new(), None);
        Self {
            fonts,
            previews: HashMap::new(),
        }
    }
}

impl std::fmt::Debug for AttachmentList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AttachmentList")
            .field("previews", &self.previews.len())
            .finish_non_exhaustive()
    }
}

/// A text style in Otto's body face.
fn style(size: f32, line_h: f32, color: Color) -> TextStyle {
    let mut style = TextStyle::new();
    style.set_font_size(size);
    style.set_font_families(&[styles::BODY.family, "sans-serif"]);
    style.set_font_style(FontStyle::new(
        Weight::from(400),
        Width::NORMAL,
        Slant::Upright,
    ));
    style.set_color(color);
    style.set_height(line_h / size);
    style.set_height_override(true);
    style
}

/// A file's name, as Files writes it in a list row.
fn name_style(color: Color) -> TextStyle {
    let face = styles::BODY_EMPHASIZED;
    let mut style = style(face.size, 18.0, color);
    style.set_font_families(&[face.family, "sans-serif"]);
    style.set_font_style(FontStyle::new(
        Weight::from(face.weight),
        Width::NORMAL,
        Slant::Upright,
    ));
    style
}

fn body(color: Color) -> TextStyle {
    style(12.5, 18.0, color)
}

fn small(color: Color) -> TextStyle {
    style(11.0, 16.0, color)
}

/// `style` with a line through it when `struck`.
fn strike(mut style: TextStyle, struck: bool) -> TextStyle {
    if struck {
        style.set_decoration_type(TextDecoration::LINE_THROUGH);
        style.set_decoration_color(style.color());
    }
    style
}

/// "2.4 MB".
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["kB", "MB", "GB", "TB"];
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1000.0;
    let mut unit = 0;
    while value >= 1000.0 && unit + 1 < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }
    if value < 10.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.0} {}", UNITS[unit])
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// `image` scaled down to fit `max_w` × `max_h` pixels.
fn shrink(image: &Image, max_w: f32, max_h: f32) -> Image {
    let (w, h) = (image.width() as f32, image.height() as f32);
    let factor = (max_w / w).min(max_h / h);
    if factor >= 1.0 {
        return image.clone();
    }
    let size = ((w * factor).round().max(1.0), (h * factor).round().max(1.0));
    let Some(mut surface) = surfaces::raster_n32_premul((size.0 as i32, size.1 as i32)) else {
        return image.clone();
    };
    surface.canvas().draw_image_rect_with_sampling_options(
        image,
        None,
        Rect::from_wh(size.0, size.1),
        SamplingOptions::from(CubicResampler::mitchell()),
        &Paint::default(),
    );
    surface.image_snapshot()
}

impl Preview {
    /// What there is to show for `path`: a screen region's picture when
    /// `region` (our own capture, decoded here), else a file's kind, size
    /// and icon, its thumbnail still to be asked for.
    fn read(path: &Path, region: bool) -> Self {
        let bytes = std::fs::metadata(path).ok().map(|meta| meta.len());
        let mut preview = Self {
            picture: None,
            bytes,
            icon: None,
            kind: None,
            thumbnail: Thumbnail::Unknown,
        };
        if region {
            if bytes.is_some_and(|b| b <= PICTURE_MAX_BYTES) {
                let decoded = std::fs::read(path)
                    .ok()
                    .and_then(|raw| Image::from_encoded(Data::new_copy(&raw)));
                preview.picture = decoded.map(|image| {
                    shrink(
                        &image,
                        PREVIEW_MAX_W * PREVIEW_SCALE,
                        PICTURE_MAX_H * PREVIEW_SCALE,
                    )
                });
            }
            preview.thumbnail = Thumbnail::None;
            return preview;
        }
        let is_dir = path.is_dir();
        let mime = if is_dir {
            None
        } else {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            filetype::mime_for_name(name).or_else(|| {
                let mut head = Vec::new();
                std::fs::File::open(path)
                    .and_then(|file| file.take(PEEK_BYTES).read_to_end(&mut head))
                    .ok()
                    .and_then(|_| filetype::sniff(&head))
            })
        };
        let (names, kind) = match mime {
            Some(mime) => (filetype::icon_names(mime), filetype::kind_of(mime)),
            None if is_dir => (vec!["folder".to_owned()], Kind::Folder),
            None => (vec![Kind::Other.generic_icon().to_owned()], Kind::Other),
        };
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let size = (ICON_SIZE * PREVIEW_SCALE) as i32;
        preview.icon = icons::cached_icon_chain_at(&names, size, icons::FULL_COLOUR_SIZE);
        preview.kind = Some(kind.label());
        if is_dir {
            // A folder's own size says nothing about what is in it, and it
            // has no thumbnail.
            preview.bytes = None;
            preview.thumbnail = Thumbnail::None;
        }
        preview
    }
}

/// Where the pieces of one item go while it is laid out.
struct Placing<'a> {
    theme: &'a Theme,
    width: f32,
    removable: bool,
    index: usize,
    struck: bool,
    pieces: Vec<Piece>,
    removes: &'a mut Vec<(Rect, usize)>,
}

impl Placing<'_> {
    /// A remove button's cross at the right edge, level with a line at `y`
    /// of height `line_h`.
    fn remove_button(&mut self, y: f32, line_h: f32) {
        if !self.removable {
            return;
        }
        let at = Point::new(self.width - CROSS, y + (line_h - CROSS) / 2.0);
        self.pieces.push(Piece::Cross(at, self.theme.text_tertiary));
        let area = Rect::from_xywh(at.x, at.y, CROSS, CROSS).with_outset((CROSS_SLOP, CROSS_SLOP));
        self.removes.push((area, self.index));
    }

    /// A remove button on a picture's top-right corner, on a dark disc so it
    /// shows over any picture.
    fn remove_button_over(&mut self, picture: Rect) {
        if !self.removable {
            return;
        }
        let disc = Rect::from_xywh(picture.right - DISC - 6.0, picture.top + 6.0, DISC, DISC);
        self.pieces.push(Piece::Box {
            rect: disc,
            radius: DISC / 2.0,
            fill: Color::from_argb(150, 0, 0, 0),
            stroke: None,
        });
        let at = Point::new(disc.center_x() - CROSS / 2.0, disc.center_y() - CROSS / 2.0);
        self.pieces
            .push(Piece::Cross(at, Color::from_argb(230, 255, 255, 255)));
        self.removes
            .push((disc.with_outset((4.0, 4.0)), self.index));
    }

    /// Room for the text beside a remove button.
    fn text_width(&self, from: f32) -> f32 {
        let button = if self.removable { CROSS + 10.0 } else { 0.0 };
        (self.width - button - from).max(40.0)
    }

    /// A picture across the list, `max_h` tall at most, at `y`.
    fn picture(&mut self, image: Image, y: f32, max_h: f32) -> Rect {
        let (w, h) = (image.width() as f32, image.height() as f32);
        let height = (self.width * h / w).clamp(PICTURE_MIN_H, max_h).round();
        let rect = Rect::from_xywh(0.0, y, self.width, height);
        self.pieces.push(Piece::Picture {
            image,
            rect,
            radius: PICTURE_RADIUS,
        });
        self.pieces.push(Piece::Box {
            rect,
            radius: PICTURE_RADIUS,
            fill: Color::TRANSPARENT,
            stroke: Some(self.theme.hairline),
        });
        rect
    }
}

impl AttachmentList {
    /// A paragraph of `spans`, one line cut with an ellipsis at `width`, or
    /// up to `max_lines` lines.
    fn text(&self, spans: &[(&str, TextStyle)], width: f32, max_lines: usize) -> Paragraph {
        let mut style = ParagraphStyle::new();
        style.set_max_lines(max_lines);
        style.set_ellipsis("…");
        let mut builder = ParagraphBuilder::new(&style, &self.fonts);
        for (text, span) in spans {
            builder.push_style(span);
            builder.add_text(text);
            builder.pop();
        }
        let mut paragraph = builder.build();
        paragraph.layout(width);
        paragraph
    }

    fn preview(&mut self, path: &Path, region: bool) -> &Preview {
        self.previews
            .entry(path.to_owned())
            .or_insert_with(|| Preview::read(path, region))
    }

    /// The files listed in the last layout whose thumbnails haven't been
    /// asked for, now counted as asked. The app finds each one, as Files
    /// does, and hands it in with [`Self::set_thumbnail`].
    pub fn thumbnails_wanted(&mut self) -> Vec<PathBuf> {
        self.previews
            .iter_mut()
            .filter(|(_, preview)| matches!(preview.thumbnail, Thumbnail::Unknown))
            .map(|(path, preview)| {
                preview.thumbnail = Thumbnail::Asked;
                path.clone()
            })
            .collect()
    }

    /// Hand in the thumbnail of `path`, or that it has none. Takes effect
    /// on the next layout.
    pub fn set_thumbnail(&mut self, path: &Path, image: Option<Image>) {
        if let Some(preview) = self.previews.get_mut(path) {
            preview.thumbnail = image.map_or(Thumbnail::None, Thumbnail::Found);
        }
    }

    /// Lay out one item at `y`. Returns how tall it is.
    fn item(&mut self, item: &Attachment, y: f32, at: &mut Placing) -> f32 {
        let theme = at.theme;
        let struck = at.struck;
        match item {
            Attachment::Text(text) => {
                let (size, line_h, lines) = (TEXT_SIZE, TEXT_LINE_H, TEXT_LINES);
                // Shown as it was selected: over the selection highlight a
                // text field draws, line by line.
                let selection = TextInputStyle::with_theme(theme.clone());
                let words: Vec<&str> = text.split_whitespace().collect();
                let quote = self.text(
                    &[(
                        &words.join(" "),
                        strike(style(size, line_h, selection.selected_text_color), struck),
                    )],
                    at.text_width(0.0),
                    lines,
                );
                let height = quote.height().ceil();
                let rects = quote
                    .get_line_metrics()
                    .iter()
                    .map(|line| {
                        let top = y + (line.baseline - line.ascent) as f32;
                        let bottom = y + (line.baseline + line.descent) as f32;
                        let left = line.left as f32;
                        Rect::from_ltrb(left, top, left + line.width as f32, bottom)
                    })
                    .collect();
                at.pieces.push(Piece::Selection {
                    rects,
                    color: selection.selection_color,
                });
                at.pieces.push(Piece::Text(quote, Point::new(0.0, y)));
                at.remove_button(y, line_h);
                height
            }
            Attachment::Region(path) => {
                let Some(image) = self.preview(path, true).picture.clone() else {
                    let missing = self.text(
                        &[(
                            crate::t!("attachments-screen-region"),
                            strike(body(theme.text_secondary), struck),
                        )],
                        at.text_width(0.0),
                        1,
                    );
                    at.pieces.push(Piece::Text(missing, Point::new(0.0, y)));
                    at.remove_button(y, 18.0);
                    return 18.0;
                };
                let rect = at.picture(image, y, PICTURE_MAX_H);
                at.remove_button_over(rect);
                rect.height()
            }
            Attachment::File(path) => {
                let preview = self.preview(path, false);
                let picture = match &preview.thumbnail {
                    Thumbnail::Found(image) => Some(image.clone()),
                    _ => None,
                };
                let icon = preview.icon.clone();
                let kind = preview.kind;
                let size = preview.bytes.map_or_else(String::new, human_size);
                let name = file_name(path);
                let icon_size = ICON_SIZE;
                let slot = Rect::from_xywh(0.0, y, icon_size, icon_size);
                // A thumbnail shows where the icon would be, as in Files.
                if let Some(image) = picture {
                    at.pieces.push(Piece::Thumbnail { image, rect: slot });
                } else if let Some(image) = icon {
                    at.pieces.push(Piece::Icon { image, rect: slot });
                }
                let x = icon_size + 12.0;
                let width = at.text_width(x);
                // Its name, and under it what it is and how big.
                let detail = [
                    kind.map(str::to_owned),
                    Some(size).filter(|s| !s.is_empty()),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ");
                let name = self.text(
                    &[(&name, strike(name_style(theme.text_primary), struck))],
                    width,
                    1,
                );
                at.pieces.push(Piece::Text(name, Point::new(x, y + 5.0)));
                let detail = self.text(&[(&detail, small(theme.text_secondary))], width, 1);
                at.pieces.push(Piece::Text(detail, Point::new(x, y + 25.0)));
                at.remove_button(y, icon_size);
                icon_size
            }
        }
    }

    /// Lay out `items`, each with whether it is struck out: kept, but not
    /// going with the request.
    pub fn layout(
        &mut self,
        items: &[(&Attachment, bool)],
        options: Options,
        theme: &Theme,
    ) -> Layout {
        self.layout_leaving(items, None, options, theme)
    }

    /// [`Self::layout`], with the item at `leaving.0` on its way out, as much
    /// of it left as `leaving.1` says, from 1 down to 0: it shrinks and fades
    /// while the items after it close up. It can't be hit.
    pub fn layout_leaving(
        &mut self,
        items: &[(&Attachment, bool)],
        leaving: Option<(usize, f32)>,
        options: Options,
        theme: &Theme,
    ) -> Layout {
        let presence = |index: usize| match leaving {
            Some((leaving, presence)) if leaving == index => presence.clamp(0.0, 1.0),
            _ => 1.0,
        };
        // Forget files that are no longer listed.
        self.previews.retain(|path, _| {
            items
                .iter()
                .any(|(item, _)| item.path() == Some(path.as_path()))
        });
        let mut pieces = Vec::new();
        let mut removes = Vec::new();
        let mut items_at = Vec::new();

        let order: Vec<usize> = if options.newest_first {
            (0..items.len()).rev().collect()
        } else {
            (0..items.len()).collect()
        };
        let mut y = 0.0;
        for (place, &index) in order.iter().enumerate() {
            if place > 0 {
                // A gap next to a leaving item closes with it.
                let closing = presence(order[place - 1]).min(presence(index));
                y += ROW_GAP * closing;
            }
            let (item, struck) = items[index];
            let mut at = Placing {
                theme,
                width: options.width,
                removable: options.removable,
                index,
                struck,
                pieces: Vec::new(),
                removes: &mut removes,
            };
            let height = self.item(item, y, &mut at);
            let placed = std::mem::take(&mut at.pieces);
            let placed = if struck {
                Piece::Faded(placed)
            } else {
                Piece::Group(placed)
            };
            let present = presence(index);
            if present < 1.0 {
                removes.retain(|&(_, removed)| removed != index);
                pieces.push(Piece::Leaving {
                    pieces: vec![placed],
                    center: Point::new(options.width / 2.0, y + height / 2.0),
                    presence: present,
                    lift: height * (1.0 - present) / 2.0,
                });
                y += height * present;
                continue;
            }
            pieces.push(placed);
            let rect =
                Rect::from_xywh(0.0, y, options.width, height).with_outset((HOVER_PAD, HOVER_PAD));
            items_at.push((rect, index));
            y += height;
        }
        Layout {
            pieces,
            removes,
            items_at,
            height: y.ceil(),
        }
    }

    /// Paint `layout` with its top-left corner at the canvas origin, the
    /// item at `hovered` highlighted.
    pub fn paint(&self, canvas: &Canvas, layout: &Layout, theme: &Theme, hovered: Option<usize>) {
        // Under the item, so its words sit on top.
        let hovered = hovered.and_then(|index| layout.items_at.iter().find(|&&(_, i)| i == index));
        if let Some((rect, _)) = hovered {
            let mut highlight = Paint::default();
            highlight.set_anti_alias(true);
            highlight.set_color(theme.fill_quaternary);
            canvas.draw_rrect(
                RRect::new_rect_xy(*rect, HOVER_RADIUS, HOVER_RADIUS),
                &highlight,
            );
        }
        paint(canvas, &layout.pieces);
    }
}

/// Paint `pieces` where they were laid out.
fn paint(canvas: &Canvas, pieces: &[Piece]) {
    // Pictures and icons draw as they are, whatever colour `fill` has.
    let mut image_paint = Paint::default();
    image_paint.set_anti_alias(true);
    let mut fill = Paint::default();
    fill.set_anti_alias(true);
    let mut stroke = fill.clone();
    stroke.set_style(PaintStyle::Stroke);
    stroke.set_stroke_width(1.0);
    for piece in pieces {
        match piece {
            Piece::Text(paragraph, at) => paragraph.paint(canvas, *at),
            Piece::Box {
                rect,
                radius,
                fill: color,
                stroke: edge,
            } => {
                if color.a() > 0 {
                    fill.set_color(*color);
                    canvas.draw_rrect(RRect::new_rect_xy(*rect, *radius, *radius), &fill);
                }
                if let Some(edge) = edge {
                    stroke.set_color(*edge);
                    let rect = rect.with_inset((0.5, 0.5));
                    canvas.draw_rrect(RRect::new_rect_xy(rect, *radius, *radius), &stroke);
                }
            }
            Piece::Picture {
                image,
                rect,
                radius,
            } => {
                // Cover the rect: crop the picture to its shape.
                let (w, h) = (image.width() as f32, image.height() as f32);
                let factor = (rect.width() / w).max(rect.height() / h);
                let (src_w, src_h) = (rect.width() / factor, rect.height() / factor);
                let src = Rect::from_xywh((w - src_w) / 2.0, (h - src_h) / 2.0, src_w, src_h);
                canvas.save();
                canvas.clip_rrect(
                    RRect::new_rect_xy(*rect, *radius, *radius),
                    ClipOp::Intersect,
                    true,
                );
                canvas.draw_image_rect_with_sampling_options(
                    image,
                    Some((&src, SrcRectConstraint::Fast)),
                    rect,
                    SamplingOptions::from(CubicResampler::mitchell()),
                    &image_paint,
                );
                canvas.restore();
            }
            Piece::Icon { image, rect } => {
                canvas.draw_image_rect_with_sampling_options(
                    image,
                    None,
                    rect,
                    SamplingOptions::from(CubicResampler::mitchell()),
                    &image_paint,
                );
            }
            Piece::Selection { rects, color } => {
                fill.set_color(*color);
                for rect in rects {
                    canvas.draw_rect(rect, &fill);
                }
            }
            Piece::Thumbnail { image, rect } => {
                let (w, h) = (image.width() as f32, image.height() as f32);
                if w <= 0.0 || h <= 0.0 {
                    continue;
                }
                // Kept at PREVIEW_SCALE pixels per point.
                let scale = (rect.width() / w)
                    .min(rect.height() / h)
                    .min(PREVIEW_SCALE.recip());
                let (dst_w, dst_h) = (w * scale, h * scale);
                let dst = Rect::from_xywh(
                    rect.center_x() - dst_w / 2.0,
                    rect.bottom - dst_h,
                    dst_w,
                    dst_h,
                );
                canvas.draw_image_rect_with_sampling_options(
                    image,
                    None,
                    dst,
                    SamplingOptions::from(CubicResampler::mitchell()),
                    &image_paint,
                );
                stroke.set_color(Color::from_argb(46, 0, 0, 0));
                canvas.draw_rect(dst.with_inset((0.5, 0.5)), &stroke);
            }
            Piece::Cross(at, color) => {
                let mut cross = stroke.clone();
                cross.set_color(*color);
                cross.set_stroke_width(1.4);
                cross.set_stroke_cap(skia_safe::paint::Cap::Round);
                let inset = CROSS * 0.25;
                let (l, t) = (at.x + inset, at.y + inset);
                let (r, b) = (at.x + CROSS - inset, at.y + CROSS - inset);
                canvas.draw_line((l, t), (r, b), &cross);
                canvas.draw_line((r, t), (l, b), &cross);
            }
            Piece::Group(pieces) => paint(canvas, pieces),
            Piece::Faded(pieces) => {
                canvas.save_layer_alpha_f(None, STRUCK_ALPHA);
                paint(canvas, pieces);
                canvas.restore();
            }
            Piece::Leaving {
                pieces,
                center,
                presence,
                lift,
            } => {
                canvas.save_layer_alpha_f(None, *presence);
                canvas.translate((center.x, center.y - lift));
                canvas.scale((*presence, *presence));
                canvas.translate((-center.x, -center.y));
                paint(canvas, pieces);
                canvas.restore();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(newest_first: bool) -> Options {
        Options {
            width: 320.0,
            newest_first,
            removable: true,
        }
    }

    #[test]
    fn a_leaving_item_closes_up_and_cannot_be_hit() {
        let mut list = AttachmentList::default();
        let (one, two) = (
            Attachment::Text("one".into()),
            Attachment::Text("two".into()),
        );
        let items = [(&one, false), (&two, false)];
        let whole = list.layout(&items, options(false), &Theme::dark());
        let half = list.layout_leaving(&items, Some((0, 0.5)), options(false), &Theme::dark());
        let gone = list.layout_leaving(&items, Some((0, 0.0)), options(false), &Theme::dark());
        let alone = list.layout(&items[1..], options(false), &Theme::dark());
        assert!(half.height < whole.height && half.height > gone.height);
        assert_eq!(gone.height, alone.height);
        let points = (0..64).flat_map(|x| (0..64).map(move |y| (x as f32 * 5.0, y as f32 * 2.0)));
        for (x, y) in points {
            assert_ne!(half.item_at(x, y), Some(0));
            assert_ne!(half.remove_at(x, y), Some(0));
        }
        assert!(half.item_at(20.0, half.height - 2.0) == Some(1));
    }

    #[test]
    fn thumbnails_are_asked_for_once_and_shown_when_handed_in() {
        let dir = std::env::temp_dir().join(format!("otto-kit-thumbs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("photo.png");
        std::fs::write(&file, b"not really a png").unwrap();
        let item = Attachment::File(file.clone());
        let mut list = AttachmentList::default();
        let theme = Theme::dark();
        list.layout(&[(&item, false)], options(true), &theme);
        // Asked for once, whatever it turns out to be; never decoded here.
        assert_eq!(list.thumbnails_wanted(), std::slice::from_ref(&file));
        assert!(list.thumbnails_wanted().is_empty());
        let icon = list.layout(&[(&item, false)], options(true), &theme);
        let pixel = {
            let info = skia_safe::ImageInfo::new(
                (1, 1),
                skia_safe::ColorType::RGBA8888,
                skia_safe::AlphaType::Premul,
                None,
            );
            skia_safe::images::raster_from_data(&info, Data::new_copy(&[0, 0, 0, 255]), 4).unwrap()
        };
        list.set_thumbnail(&file, Some(pixel));
        let thumbnail = list.layout(&[(&item, false)], options(true), &theme);
        let has_thumbnail = |layout: &Layout| {
            fn any(pieces: &[Piece]) -> bool {
                pieces.iter().any(|piece| match piece {
                    Piece::Thumbnail { .. } => true,
                    Piece::Group(inner) | Piece::Faded(inner) => any(inner),
                    _ => false,
                })
            }
            any(&layout.pieces)
        };
        assert!(!has_thumbnail(&icon));
        assert!(has_thumbnail(&thumbnail));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn sizes_read_like_files_do() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(2_400_000), "2.4 MB");
        assert_eq!(human_size(31_000), "31 kB");
    }

    #[test]
    fn the_newest_leads_and_every_item_can_be_hit() {
        let mut list = AttachmentList::default();
        let (one, two) = (
            Attachment::Text("one".into()),
            Attachment::Text("two".into()),
        );
        let layout = list.layout(
            &[(&one, false), (&two, false)],
            options(true),
            &Theme::dark(),
        );
        // The newest is first.
        let (newest, index) = layout.removes[0];
        assert_eq!(index, 1);
        assert_eq!(
            layout.remove_at(newest.center_x(), newest.center_y()),
            Some(1)
        );
        let (older, _) = layout.items_at[1];
        assert_eq!(layout.item_at(older.left + 20.0, older.center_y()), Some(0));
        assert_eq!(layout.item_at(-50.0, -50.0), None);
    }

    #[test]
    fn every_item_is_the_same_size_wherever_it_is() {
        let mut list = AttachmentList::default();
        let (one, two) = (
            Attachment::Text("one".into()),
            Attachment::Text("two".into()),
        );
        let layout = list.layout(
            &[(&one, false), (&two, false)],
            options(true),
            &Theme::dark(),
        );
        let (first, second) = (layout.items_at[0].0, layout.items_at[1].0);
        assert_eq!(first.height(), second.height());
    }

    #[test]
    fn a_record_keeps_its_order_and_has_no_buttons() {
        let mut list = AttachmentList::default();
        let (one, two) = (
            Attachment::Text("one".into()),
            Attachment::Text("two".into()),
        );
        let options = Options {
            removable: false,
            ..options(false)
        };
        let layout = list.layout(&[(&one, false), (&two, false)], options, &Theme::dark());
        assert!(layout.removes.is_empty());
        assert_eq!(layout.items_at[0].1, 0);
        assert!(layout.items_at[0].0.top < layout.items_at[1].0.top);
    }

    #[test]
    fn gathered_files_read_as_what_they_are() {
        let gather = std::env::temp_dir().join(format!("attachments-test-{}", std::process::id()));
        let dir = gather.join("123");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("selection-1.txt"), "hello").unwrap();
        assert_eq!(
            Attachment::for_file_in(&dir.join("selection-1.txt"), &gather),
            Attachment::Text("hello".into())
        );
        assert_eq!(
            Attachment::for_file_in(&gather.join("region-9.png"), &gather),
            Attachment::Region(gather.join("region-9.png"))
        );
        assert_eq!(
            Attachment::for_file_in(Path::new("/tmp/selection-1.txt"), &gather),
            Attachment::File("/tmp/selection-1.txt".into())
        );
        std::fs::remove_dir_all(&gather).unwrap();
    }
}
