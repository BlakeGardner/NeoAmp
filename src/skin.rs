//! Classic Winamp skin (.wsz) loading: sprite sheets, viscolor.txt, pledit.txt, region.txt and
//! the mouse pointers (`.cur` files, see [`crate::cursor`]). A sheet may also come as SVG
//! (`cbuttons.svg`), which NeoAmp draws in place of the bitmap at the windows' density.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Cursor, Read};
use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::canvas::{Canvas, Density, Image, Rect};
use crate::cursor::{self, CursorImage, SkinCursor};
use crate::sprites::{Sheet, Sprite};
use crate::svg::SvgSheet;

/// Largest sheet dimension we accept from an untrusted archive (real sheets are under 400px).
const MAX_SHEET_SIDE: u32 = 4096;
/// Largest `.wsz` file read, from disk or the network. Skins are a few hundred kilobytes; the
/// largest in the Winamp Skin Museum, a few megabytes.
pub const MAX_WSZ_BYTES: usize = 32 << 20;
/// Most entries an archive may list. Skins have a few dozen.
const MAX_ENTRIES: usize = 4096;
/// Most bytes one file in an archive may unpack to: room for any real sheet or SVG.
const MAX_ENTRY_BYTES: usize = 8 << 20;
/// Most bytes the files NeoAmp reads from an archive may unpack to, together.
const MAX_UNPACKED_BYTES: usize = 32 << 20;
/// The text files read from a skin.
const TEXT_FILES: [&str; 3] = ["viscolor.txt", "pledit.txt", "region.txt"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PleditColors {
    pub normal: [u8; 4],
    pub current: [u8; 4],
    pub normal_bg: [u8; 4],
    pub selected_bg: [u8; 4],
    pub font: String,
}

impl Default for PleditColors {
    fn default() -> Self {
        Self {
            normal: [0x00, 0xFF, 0x00, 0xFF],
            current: [0xFF, 0xFF, 0xFF, 0xFF],
            normal_bg: [0x00, 0x00, 0x00, 0xFF],
            selected_bg: [0x00, 0x00, 0xC6, 0xFF],
            font: "Arial".to_string(),
        }
    }
}

/// Polygons in window pixel-edge coordinates; a pixel is visible if its centre is inside any polygon.
pub type Polygons = Vec<Vec<(f32, f32)>>;

/// The window shapes `region.txt` gives, one per section.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Regions {
    /// `[Normal]`: the main window.
    pub normal: Option<Polygons>,
    /// `[WindowShade]`: the main window in windowshade mode.
    pub windowshade: Option<Polygons>,
    /// `[Equalizer]`.
    pub equalizer: Option<Polygons>,
    /// `[EqualizerWS]`: the equalizer in windowshade mode.
    pub equalizer_ws: Option<Polygons>,
}

pub const DEFAULT_VIS_COLORS: [[u8; 4]; 24] = [
    [0, 0, 0, 255],
    [24, 33, 41, 255],
    [239, 49, 16, 255],
    [206, 41, 16, 255],
    [214, 90, 0, 255],
    [214, 102, 0, 255],
    [214, 115, 0, 255],
    [198, 123, 8, 255],
    [222, 165, 24, 255],
    [214, 181, 33, 255],
    [189, 222, 41, 255],
    [148, 222, 33, 255],
    [41, 206, 16, 255],
    [50, 190, 16, 255],
    [57, 181, 16, 255],
    [49, 156, 8, 255],
    [41, 148, 0, 255],
    [24, 132, 8, 255],
    [255, 255, 255, 255],
    [214, 214, 222, 255],
    [181, 189, 189, 255],
    [160, 170, 175, 255],
    [148, 156, 165, 255],
    [150, 150, 150, 255],
];

const SHEET_FILES: &[(Sheet, &str)] = &[
    (Sheet::Main, "main"),
    (Sheet::TitleBar, "titlebar"),
    (Sheet::CButtons, "cbuttons"),
    (Sheet::ShufRep, "shufrep"),
    (Sheet::PosBar, "posbar"),
    (Sheet::Volume, "volume"),
    (Sheet::Balance, "balance"),
    (Sheet::MonoSter, "monoster"),
    (Sheet::PlayPaus, "playpaus"),
    (Sheet::Numbers, "numbers"),
    (Sheet::NumsEx, "nums_ex"),
    (Sheet::Text, "text"),
    (Sheet::EqMain, "eqmain"),
    (Sheet::EqEx, "eq_ex"),
    (Sheet::PlEdit, "pledit"),
    (Sheet::Gen, "gen"),
];

/// A loaded classic skin: decoded sprite sheets plus the parsed text configuration.
#[derive(Clone)]
pub struct Skin {
    pub name: String,
    sheets: HashMap<Sheet, Canvas>,
    /// Sheets drawn as SVG, in place of the bitmaps of the same name.
    vectors: HashMap<Sheet, SvgSheet>,
    /// Sheets taken from the base skin because this one has none of its own.
    borrowed: Vec<Sheet>,
    empty: Canvas,
    pub vis_colors: [[u8; 4]; 24],
    pub pledit: PleditColors,
    /// The window shapes; the windows are cut to them as they are drawn, at whatever density.
    pub regions: Regions,
    /// The mouse pointers the skin has; the system's show where it has none.
    cursors: HashMap<SkinCursor, CursorImage>,
}

impl std::fmt::Debug for Skin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let has_regions = self.regions != Regions::default();
        f.debug_struct("Skin")
            .field("name", &self.name)
            .field("sheets", &self.sheets.keys().collect::<Vec<_>>())
            .field("svg", &self.vectors.keys().collect::<Vec<_>>())
            .field("borrowed", &self.borrowed)
            .field("pledit", &self.pledit)
            .field("has_regions", &has_regions)
            .field("cursors", &self.cursors.len())
            .finish_non_exhaustive()
    }
}

impl Skin {
    /// Load a `.wsz` file from disk.
    ///
    /// # Errors
    /// Fails if the file cannot be read, is larger than [`MAX_WSZ_BYTES`], is not a zip
    /// archive, or contains no usable sheets.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Skin> {
        let path = path.as_ref();
        let mut bytes = Vec::new();
        File::open(path)
            .and_then(|f| f.take(limit(MAX_WSZ_BYTES)).read_to_end(&mut bytes))
            .with_context(|| format!("reading {}", path.display()))?;
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "skin".into());
        Self::from_wsz(&bytes, &name)
    }

    /// Parse a `.wsz` archive held in memory. Only the files a skin is made of are unpacked.
    ///
    /// # Errors
    /// Fails if `bytes` is not a zip archive or contains no usable sheets, or if the archive
    /// is larger than [`MAX_WSZ_BYTES`], lists more than 4096 files or its skin files unpack
    /// to more than 32 MB. Individual undecodable entries are logged and skipped, as Winamp
    /// did, and so is a file that unpacks to more than 8 MB.
    pub fn from_wsz(bytes: &[u8], name: &str) -> Result<Skin> {
        if bytes.len() > MAX_WSZ_BYTES {
            bail!(
                "{name}: more than {} MB, too large for a skin",
                MAX_WSZ_BYTES >> 20
            );
        }
        let mut archive =
            zip::ZipArchive::new(Cursor::new(bytes)).context("skin is not a zip archive")?;
        if archive.len() > MAX_ENTRIES {
            bail!(
                "{name}: {} files in the archive, too many for a skin",
                archive.len()
            );
        }
        let mut files: HashMap<String, Vec<u8>> = HashMap::new();
        let mut unpacked = 0;
        for i in 0..archive.len() {
            let f = match archive.by_index(i) {
                Ok(f) => f,
                Err(e) => {
                    log::warn!("skipping zip entry {i}: {e}");
                    continue;
                }
            };
            if !f.is_file() {
                continue;
            }
            let full_name = match f.name() {
                Ok(n) => n.into_owned(),
                Err(_) => continue,
            };
            let base = full_name
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            if files.contains_key(&base) || !is_skin_file(&base) {
                continue;
            }
            let mut buf = Vec::new();
            if f.take(limit(MAX_ENTRY_BYTES))
                .read_to_end(&mut buf)
                .is_err()
            {
                continue;
            }
            if buf.len() > MAX_ENTRY_BYTES {
                log::warn!("{name}: skipping {full_name}, larger than a skin's files are");
                continue;
            }
            unpacked += buf.len();
            if unpacked > MAX_UNPACKED_BYTES {
                bail!(
                    "{name}: unpacks to more than {} MB, too much for a skin",
                    MAX_UNPACKED_BYTES >> 20
                );
            }
            files.insert(base, buf);
        }
        Self::from_files(&files, name)
    }

    fn from_files(files: &HashMap<String, Vec<u8>>, name: &str) -> Result<Skin> {
        let mut sheets = HashMap::with_capacity(SHEET_FILES.len());
        for (sheet, base) in SHEET_FILES {
            for ext in ["bmp", "png"] {
                let Some(bytes) = files.get(&format!("{base}.{ext}")) else {
                    continue;
                };
                match decode_image(bytes) {
                    Ok(img) => {
                        sheets.insert(*sheet, img);
                        break;
                    }
                    Err(e) => log::warn!("{name}: cannot decode {base}.{ext}: {e}"),
                }
            }
        }
        let mut vectors = HashMap::new();
        for (sheet, base) in SHEET_FILES {
            let Some(bytes) = files.get(&format!("{base}.svg")) else {
                continue;
            };
            match SvgSheet::parse(bytes) {
                Ok(svg) => {
                    vectors.insert(*sheet, svg);
                }
                Err(e) => log::warn!("{name}: cannot read {base}.svg: {e:#}"),
            }
        }
        if sheets.is_empty() && vectors.is_empty() {
            bail!("{name}: no skin bitmaps or SVG sheets found in archive");
        }
        let text = |key: &str| files.get(key).map(|b| String::from_utf8_lossy(b));
        let vis_colors = text("viscolor.txt").map_or(DEFAULT_VIS_COLORS, |t| parse_viscolor(&t));
        let pledit = text("pledit.txt")
            .map(|t| parse_pledit(&t))
            .unwrap_or_default();
        let regions = text("region.txt")
            .map(|t| parse_region(&t))
            .unwrap_or_default();
        let mut cursors = HashMap::new();
        for which in SkinCursor::ALL {
            let Some(bytes) = files.get(which.file_name()) else {
                continue;
            };
            match cursor::parse(bytes) {
                Ok(image) => {
                    cursors.insert(which, image);
                }
                Err(e) => log::warn!("{name}: cannot decode {}: {e:#}", which.file_name()),
            }
        }
        Ok(Skin {
            name: name.to_string(),
            sheets,
            vectors,
            borrowed: Vec::new(),
            empty: Canvas::from(Image::new(0, 0)),
            vis_colors,
            pledit,
            regions,
            cursors,
        })
    }

    /// Fill in sheets and pointers this skin is missing from `base` (Winamp falls back per
    /// file).
    pub fn fill_missing_from(&mut self, base: &Skin) {
        for (which, image) in &base.cursors {
            self.cursors.entry(*which).or_insert_with(|| image.clone());
        }
        // Skins without balance.bmp reuse the volume slider graphics.
        if !self.has_sheet(Sheet::Balance) {
            if let Some(v) = self.sheets.get(&Sheet::Volume).cloned() {
                self.sheets.insert(Sheet::Balance, v);
            }
            if let Some(v) = self.vectors.get(&Sheet::Volume).cloned() {
                self.vectors.insert(Sheet::Balance, v);
            }
        }
        // A sheet the skin has in neither form comes from the base skin, in both.
        let missing: HashSet<Sheet> = base
            .sheets
            .keys()
            .chain(base.vectors.keys())
            // nums_ex is optional; its absence selects the numbers.bmp path.
            .filter(|k| **k != Sheet::NumsEx && !self.has_sheet(**k))
            .copied()
            .collect();
        for k in missing {
            if let Some(v) = base.sheets.get(&k) {
                self.sheets.insert(k, v.clone());
            }
            if let Some(v) = base.vectors.get(&k) {
                self.vectors.insert(k, v.clone());
            }
            self.borrowed.push(k);
        }
    }

    /// The skin's pointer for `which`, if it has one.
    #[must_use]
    pub fn cursor(&self, which: SkinCursor) -> Option<&CursorImage> {
        self.cursors.get(&which)
    }

    /// Whether the skin has the sheet, as a bitmap or as SVG.
    #[must_use]
    pub fn has_sheet(&self, s: Sheet) -> bool {
        self.sheets.contains_key(&s) || self.vectors.contains_key(&s)
    }

    /// Whether the sheet is drawn from SVG.
    #[must_use]
    pub fn has_svg(&self, s: Sheet) -> bool {
        self.vectors.contains_key(&s)
    }

    /// Whether the skin brought this sheet itself, rather than borrowing the base skin's.
    #[must_use]
    pub fn has_own_sheet(&self, s: Sheet) -> bool {
        self.has_sheet(s) && !self.borrowed.contains(&s)
    }

    /// The sheet to draw on a canvas of `density`: the SVG sheet rasterised for it, else the
    /// bitmap, else an empty canvas (so blits draw nothing).
    #[must_use]
    pub fn sheet_at(&self, s: Sheet, density: Density) -> &Canvas {
        self.vectors
            .get(&s)
            .and_then(|v| v.raster(density))
            .or_else(|| self.sheets.get(&s))
            .unwrap_or(&self.empty)
    }

    /// The sheet at one image pixel per skin pixel, for reading its colours.
    #[must_use]
    pub fn sheet(&self, s: Sheet) -> &Canvas {
        self.sheet_at(s, Density::ONE)
    }

    pub fn blit(&self, dst: &mut Canvas, s: Sprite, x: i32, y: i32) {
        let sheet = self.sheet_at(s.sheet, dst.density());
        dst.blit(sheet, s.rect(), x, y);
    }

    pub fn blit_clipped(&self, dst: &mut Canvas, s: Sprite, x: i32, y: i32, clip: Rect) {
        let sheet = self.sheet_at(s.sheet, dst.density());
        dst.blit_clipped(sheet, s.rect(), x, y, clip);
    }
}

/// A reader limit one byte past `max`, so that going over it shows.
fn limit(max: usize) -> u64 {
    u64::try_from(max).map_or(u64::MAX, |m| m.saturating_add(1))
}

/// Whether `name` (lowercase, without folders) is one of the files a skin is made of.
fn is_skin_file(name: &str) -> bool {
    if TEXT_FILES.contains(&name) || SkinCursor::ALL.iter().any(|c| c.file_name() == name) {
        return true;
    }
    name.rsplit_once('.').is_some_and(|(stem, ext)| {
        matches!(ext, "bmp" | "png" | "svg") && SHEET_FILES.iter().any(|(_, s)| *s == stem)
    })
}

fn decode_image(bytes: &[u8]) -> Result<Canvas> {
    let reader = || image::ImageReader::new(Cursor::new(bytes)).with_guessed_format();
    // The size first, so that a small file claiming a huge picture is never decoded.
    let (w, h) = reader()?.into_dimensions()?;
    if w == 0 || h == 0 || w > MAX_SHEET_SIDE || h > MAX_SHEET_SIDE {
        bail!("unreasonable sheet size {w}x{h}");
    }
    let img = reader()?.decode()?;
    let (w, h) = (img.width(), img.height());
    let mut data = img.into_rgba8().into_raw();
    // Skins are opaque bitmaps; normalise alpha so blits never leak transparency.
    for px in data.chunks_exact_mut(4) {
        px[3] = 255;
    }
    Image::from_rgba(i32::try_from(w)?, i32::try_from(h)?, data)
        .map(Canvas::from)
        .context("pixel buffer size mismatch")
}

fn strip_comment(line: &str) -> &str {
    let mut s = line;
    if let Some(i) = s.find("//") {
        s = &s[..i];
    }
    if let Some(i) = s.find(';') {
        s = &s[..i];
    }
    s.trim()
}

/// Parse `viscolor.txt`: up to 24 lines of `r,g,b`, comments allowed. Missing entries keep the defaults.
#[must_use]
pub fn parse_viscolor(text: &str) -> [[u8; 4]; 24] {
    let mut colors = DEFAULT_VIS_COLORS;
    let mut n = 0;
    for line in text.lines() {
        if n >= colors.len() {
            break;
        }
        let s = strip_comment(line);
        if s.is_empty() {
            continue;
        }
        let mut parts = s
            .split([',', ' ', '\t'])
            .filter(|p| !p.is_empty())
            .filter_map(|p| p.parse::<i64>().ok())
            .map(|v| u8::try_from(v.clamp(0, 255)).unwrap_or(0));
        if let (Some(r), Some(g), Some(b)) = (parts.next(), parts.next(), parts.next()) {
            colors[n] = [r, g, b, 255];
            n += 1;
        }
    }
    colors
}

fn parse_color(s: &str) -> Option<[u8; 4]> {
    let s = s.trim().trim_start_matches('#');
    if s.len() < 6 || !s.is_char_boundary(6) {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some([r, g, b, 255])
}

/// Parse `pledit.txt` (`[Text]` section with `Normal=#RRGGBB` style keys).
#[must_use]
pub fn parse_pledit(text: &str) -> PleditColors {
    let mut c = PleditColors::default();
    for line in text.lines() {
        let Some((k, v)) = line.trim().split_once('=') else {
            continue;
        };
        let v = v.trim();
        match k.trim().to_ascii_lowercase().as_str() {
            "normal" => c.normal = parse_color(v).unwrap_or(c.normal),
            "current" => c.current = parse_color(v).unwrap_or(c.current),
            "normalbg" => c.normal_bg = parse_color(v).unwrap_or(c.normal_bg),
            "selectedbg" => c.selected_bg = parse_color(v).unwrap_or(c.selected_bg),
            "font" => c.font = v.to_string(),
            _ => {}
        }
    }
    c
}

/// Parse `region.txt`: `[Normal]`, `[WindowShade]`, `[Equalizer]` and `[EqualizerWS]` sections
/// with `NumPoints` and `PointList`. Section names are matched ignoring case, and only the
/// first definition of each counts, as in Winamp.
#[must_use]
pub fn parse_region(text: &str) -> Regions {
    let mut regions = Regions::default();
    let mut section = String::new();
    let mut num_points: Vec<usize> = Vec::new();
    let mut points: Vec<i32> = Vec::new();
    let mut flush = |section: &str, num_points: &mut Vec<usize>, points: &mut Vec<i32>| {
        let slot = match section {
            "normal" => Some(&mut regions.normal),
            "windowshade" => Some(&mut regions.windowshade),
            "equalizer" => Some(&mut regions.equalizer),
            "equalizerws" => Some(&mut regions.equalizer_ws),
            _ => None,
        };
        if let Some(slot) = slot
            && points.len() >= 6
        {
            slot.get_or_insert_with(|| build_polygons(num_points, points));
        }
        num_points.clear();
        points.clear();
    };
    for raw in text.lines() {
        let line = strip_comment(raw);
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            flush(&section, &mut num_points, &mut points);
            section = line.trim_matches(['[', ']']).trim().to_ascii_lowercase();
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let ints = v
            .split([',', ' ', '\t'])
            .filter(|p| !p.is_empty())
            .filter_map(|p| p.parse::<i32>().ok());
        match k.trim().to_ascii_lowercase().as_str() {
            "numpoints" => num_points = ints.map(|n| usize::try_from(n).unwrap_or(0)).collect(),
            "pointlist" => points = ints.collect(),
            _ => {}
        }
    }
    flush(&section, &mut num_points, &mut points);
    regions
}

fn build_polygons(num_points: &[usize], points: &[i32]) -> Polygons {
    let pts: Vec<(f32, f32)> = points
        .chunks_exact(2)
        .map(|p| (p[0] as f32, p[1] as f32))
        .collect();
    if num_points.is_empty() {
        return vec![pts];
    }
    let mut polys = Vec::with_capacity(num_points.len());
    let mut i = 0;
    for &n in num_points {
        if n < 3 || i + n > pts.len() {
            break;
        }
        polys.push(pts[i..i + n].to_vec());
        i += n;
    }
    if polys.is_empty() {
        polys.push(pts);
    }
    polys
}

/// Mark the pixels of one row that `polys` cover, a pixel being covered when its centre is
/// inside a polygon. `cy` is the row's centre in polygon units (skin pixels), `scale` how many
/// of `row`'s pixels make one unit, and `xs` scratch space for the edge crossings.
pub fn cover_row(polys: &Polygons, cy: f32, scale: f32, row: &mut [bool], xs: &mut Vec<f32>) {
    let len = row.len() as f32;
    for poly in polys.iter().filter(|p| p.len() >= 3) {
        xs.clear();
        for (i, &(x1, y1)) in poly.iter().enumerate() {
            let (x2, y2) = poly[(i + 1) % poly.len()];
            if (y1 <= cy && y2 > cy) || (y2 <= cy && y1 > cy) {
                xs.push(x1 + (cy - y1) * (x2 - x1) / (y2 - y1));
            }
        }
        xs.sort_by(f32::total_cmp);
        for pair in xs.chunks_exact(2) {
            // Clamped to 0..=len, so the casts stay in range.
            let x0 = (pair[0] * scale).round().clamp(0.0, len) as usize;
            let x1 = (pair[1] * scale).round().clamp(0.0, len) as usize;
            if x0 < x1 {
                row[x0..x1].fill(true);
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::sprites::main;

    #[test]
    fn viscolor_parses_lines_with_comments_and_keeps_defaults_for_the_rest() {
        let colors = parse_viscolor("1,2,3, // first\n; a comment line\n  4 , 5 , 6\n300,-1,7\n");
        assert_eq!(colors[0], [1, 2, 3, 255]);
        assert_eq!(colors[1], [4, 5, 6, 255]);
        assert_eq!(colors[2], [255, 0, 7, 255]);
        assert_eq!(colors[3], DEFAULT_VIS_COLORS[3]);
    }

    #[test]
    fn pledit_keys_are_case_insensitive_and_bad_values_keep_defaults() {
        let c =
            parse_pledit("[Text]\nNORMAL=#102030\ncurrent=404040\nNormalBG=oops\nFont=Tahoma\n");
        assert_eq!(c.normal, [0x10, 0x20, 0x30, 255]);
        assert_eq!(c.current, [0x40, 0x40, 0x40, 255]);
        assert_eq!(c.normal_bg, PleditColors::default().normal_bg);
        assert_eq!(c.font, "Tahoma");
    }

    /// `polys` rasterised at one pixel per skin pixel into a `w`x`h` grid, `true` where covered.
    fn region_mask(polys: &Polygons, w: usize, h: usize) -> Vec<bool> {
        let mut mask = vec![false; w * h];
        let mut xs = Vec::new();
        for (y, row) in mask.chunks_exact_mut(w).enumerate() {
            cover_row(polys, y as f32 + 0.5, 1.0, row, &mut xs);
        }
        mask
    }

    #[test]
    fn region_rectangles_rasterise_to_their_exact_pixel_area() {
        let r = parse_region(
            "; comment\n[Normal]\nNumPoints=4,4\nPointList=0,0, 4,0, 4,2, 0,2,   6,0, 8,0, 8,1, 6,1\n[Equalizer]\nNumPoints=4\nPointList=1,1,3,1,3,3,1,3\n",
        );
        let main = region_mask(r.normal.as_ref().unwrap(), 10, 4);
        assert_eq!(main.iter().filter(|v| **v).count(), 4 * 2 + 2);
        assert!(main[0] && main[3] && !main[4] && main[6] && main[7] && !main[8]);
        assert!(!main[2 * 10]);
        let eq = region_mask(r.equalizer.as_ref().unwrap(), 4, 4);
        assert_eq!(eq.iter().filter(|v| **v).count(), 4);
        assert!(eq[4 + 1] && !eq[0]);
    }

    #[test]
    fn windowshade_regions_are_read_and_only_the_first_of_each_counts() {
        let r = parse_region(
            "[WINDOWSHADE]\r\nNumPoints=4\r\nPointList=0,0,275,0,275,14,0,14\r\n\
             [Unknown]\r\nNumPoints=4\r\nPointList=0,0,9,0,9,9,0,9\r\n\
             [EqualizerWS]\r\nNumPoints=4\r\nPointList=1,1,274,1,274,13,1,13\r\n\
             [WindowShade]\r\nNumPoints=4\r\nPointList=0,0,1,0,1,1,0,1\r\n",
        );
        assert_eq!(r.normal, None);
        assert_eq!(r.equalizer, None);
        let shade = region_mask(r.windowshade.as_ref().unwrap(), 275, 14);
        assert!(
            shade.iter().all(|v| *v),
            "the second [WindowShade] is ignored"
        );
        let eq = region_mask(r.equalizer_ws.as_ref().unwrap(), 275, 14);
        assert_eq!(eq.iter().filter(|v| **v).count(), 273 * 12);
        // A section without its own PointList takes nothing from the section before it.
        let r = parse_region("[Unknown]\nPointList=0,0,4,0,4,4\n[Normal]\nNumPoints=3\n");
        assert_eq!(r, Regions::default());
    }

    #[test]
    fn region_file_with_only_comments_defines_nothing() {
        let r = parse_region(";[Normal]\n;NumPoints=4\n;PointList=0,0,1,0,1,1,0,1\n");
        assert_eq!(r, Regions::default());
    }

    #[test]
    fn embedded_base_skin_loads_with_every_sheet() {
        let skin = Skin::from_wsz(crate::app::BASE_SKIN, "base").unwrap();
        for (sheet, _) in SHEET_FILES {
            if *sheet != Sheet::NumsEx {
                assert!(skin.has_sheet(*sheet), "missing {sheet:?}");
            }
        }
        assert_eq!(skin.sheet(Sheet::Main).size(), (main::W, main::H));
        assert!(skin.regions.normal.is_none() && skin.regions.windowshade.is_none());
        assert!(skin.has_own_sheet(Sheet::EqEx));
        // Winamp 2.91's own pointers, and only those.
        let has: Vec<SkinCursor> = SkinCursor::ALL
            .into_iter()
            .filter(|c| skin.cursor(*c).is_some())
            .collect();
        assert_eq!(
            has,
            [
                SkinCursor::Close,
                SkinCursor::EqSlid,
                SkinCursor::EqTitle,
                SkinCursor::MainMenu,
                SkinCursor::PosBar,
                SkinCursor::PSize,
                SkinCursor::TitleBar,
                SkinCursor::VolBal,
            ]
        );
    }

    #[test]
    fn embedded_neoamp_skin_is_all_svg_and_borrows_only_the_pointers() {
        let base = Skin::from_wsz(crate::app::BASE_SKIN, "base").unwrap();
        let mut skin = Skin::from_wsz(crate::app::NEOAMP_SKIN, "NeoAmp").unwrap();
        skin.fill_missing_from(&base);
        for (sheet, _) in SHEET_FILES {
            // MilkDrop's frame comes from the base skin until NeoAmp's has one of its own.
            if *sheet != Sheet::NumsEx && *sheet != Sheet::Gen {
                assert!(skin.has_svg(*sheet), "{sheet:?} is not SVG");
                assert!(skin.has_own_sheet(*sheet), "{sheet:?} is borrowed");
            }
        }
        assert!(skin.has_sheet(Sheet::Gen) && !skin.has_own_sheet(Sheet::Gen));
        assert!(!skin.has_sheet(Sheet::NumsEx));
        assert_eq!(skin.sheet(Sheet::Main).size(), (main::W, main::H));
        for which in SkinCursor::ALL {
            assert_eq!(skin.cursor(which), base.cursor(which), "{which:?}");
        }
    }

    #[test]
    fn missing_sheets_are_borrowed_from_the_base_skin_and_known_as_borrowed() {
        let base = Skin::from_wsz(crate::app::BASE_SKIN, "base").unwrap();
        let mut files = HashMap::new();
        let main_bmp = image_bmp(main::W, main::H);
        files.insert("main.bmp".to_string(), main_bmp);
        files.insert("normal.cur".to_string(), b"damaged".to_vec());
        let mut skin = Skin::from_files(&files, "partial").unwrap();
        assert!(skin.cursor(SkinCursor::TitleBar).is_none());
        skin.fill_missing_from(&base);
        assert_eq!(
            skin.cursor(SkinCursor::TitleBar),
            base.cursor(SkinCursor::TitleBar),
            "pointers are borrowed too"
        );
        assert!(
            skin.cursor(SkinCursor::Normal).is_none(),
            "a damaged one is skipped"
        );
        assert!(skin.has_own_sheet(Sheet::Main));
        assert!(skin.has_sheet(Sheet::EqEx) && !skin.has_own_sheet(Sheet::EqEx));
        assert!(!skin.has_sheet(Sheet::NumsEx), "never borrowed");
    }

    /// A black BMP file of `w` x `h`.
    fn image_bmp(w: i32, h: i32) -> Vec<u8> {
        let img = image::RgbImage::new(w.unsigned_abs(), h.unsigned_abs());
        let mut out = Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Bmp).unwrap();
        out.into_inner()
    }

    /// A `w` x `h` SVG sheet: black, with its left half red.
    fn half_red_svg(w: i32, h: i32) -> Vec<u8> {
        format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}"><rect width="{w}" height="{h}"/><rect width="{}" height="{h}" fill="#f00"/></svg>"##,
            w / 2
        )
        .into_bytes()
    }

    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLACK: [u8; 4] = [0, 0, 0, 255];

    #[test]
    fn svg_sheets_take_the_place_of_bitmaps_at_every_density() {
        let files = HashMap::from([
            ("main.bmp".to_string(), image_bmp(main::W, main::H)),
            ("cbuttons.bmp".to_string(), image_bmp(136, 36)),
            ("cbuttons.svg".to_string(), half_red_svg(136, 36)),
            ("titlebar.svg".to_string(), half_red_svg(344, 87)),
        ]);
        let mut skin = Skin::from_files(&files, "neo").unwrap();
        assert!(skin.has_svg(Sheet::CButtons) && skin.has_svg(Sheet::TitleBar));
        assert!(!skin.has_svg(Sheet::Main));
        assert_eq!(skin.sheet(Sheet::CButtons).get(10, 10), RED, "the SVG wins");
        let dense = skin.sheet_at(Sheet::CButtons, Density::new(3).unwrap());
        assert_eq!((dense.size(), dense.image().w), ((136, 36), 408));
        assert_eq!(dense.get(100, 10), BLACK);
        // Bitmaps keep being drawn at any density, enlarged as they are blitted.
        let main_sheet = skin.sheet_at(Sheet::Main, Density::new(3).unwrap());
        assert_eq!(main_sheet.density(), Density::ONE);
        // The base skin fills in only what the skin has in neither form.
        let base = Skin::from_wsz(crate::app::BASE_SKIN, "base").unwrap();
        skin.fill_missing_from(&base);
        assert!(
            skin.has_own_sheet(Sheet::TitleBar),
            "an SVG-only sheet is the skin's own"
        );
        assert!(skin.has_own_sheet(Sheet::CButtons));
        assert!(skin.has_sheet(Sheet::EqMain) && !skin.has_own_sheet(Sheet::EqMain));
        assert_eq!(skin.sheet(Sheet::TitleBar).get(300, 10), BLACK);
    }

    #[test]
    fn a_skin_of_svg_sheets_alone_loads_and_shares_its_volume_with_balance() {
        let files = HashMap::from([("volume.svg".to_string(), half_red_svg(68, 433))]);
        let mut skin = Skin::from_files(&files, "vector").unwrap();
        let base = Skin::from_wsz(crate::app::BASE_SKIN, "base").unwrap();
        skin.fill_missing_from(&base);
        assert!(skin.has_svg(Sheet::Balance) && skin.has_own_sheet(Sheet::Balance));
        assert_eq!(skin.sheet(Sheet::Balance).get(5, 5), RED);
    }

    #[test]
    fn a_broken_svg_sheet_falls_back_to_its_bitmap() {
        let files = HashMap::from([
            ("cbuttons.bmp".to_string(), image_bmp(136, 36)),
            ("cbuttons.svg".to_string(), b"<svg oops".to_vec()),
            ("shufrep.svg".to_string(), half_red_svg(5000, 85)),
        ]);
        let skin = Skin::from_files(&files, "broken").unwrap();
        assert!(!skin.has_svg(Sheet::CButtons) && !skin.has_svg(Sheet::ShufRep));
        assert_eq!(skin.sheet(Sheet::CButtons).get(10, 10), BLACK);
        assert!(
            Skin::from_files(
                &HashMap::from([("main.svg".to_string(), b"x".to_vec())]),
                "none"
            )
            .is_err()
        );
    }

    #[test]
    fn garbage_is_rejected_not_panicked_on() {
        assert!(Skin::from_wsz(b"not a zip", "x").is_err());
    }

    /// A zip archive of `files`, deflated.
    fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        use std::io::Write as _;
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .compression_level(Some(1));
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, bytes) in files {
            zip.start_file(*name, options).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn archives_listing_too_many_files_are_refused() {
        let names: Vec<String> = (0..=MAX_ENTRIES).map(|i| format!("{i}.txt")).collect();
        let files: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), &b""[..])).collect();
        let e = Skin::from_wsz(&zip_of(&files), "many").unwrap_err();
        assert!(e.to_string().contains("too many"), "{e}");
    }

    #[test]
    fn only_skin_files_are_unpacked_and_an_oversized_one_is_skipped() {
        let main_bmp = image_bmp(main::W, main::H);
        let huge = vec![0; MAX_ENTRY_BYTES + 1];
        let wsz = zip_of(&[
            ("Skin/main.bmp", &main_bmp),
            ("Skin/cbuttons.bmp", &huge),
            // Not a skin file: never unpacked, so it counts towards nothing.
            ("Skin/video.bmp", &vec![0; MAX_UNPACKED_BYTES + 1]),
        ]);
        let skin = Skin::from_wsz(&wsz, "padded").unwrap();
        assert!(skin.has_sheet(Sheet::Main));
        assert!(!skin.has_sheet(Sheet::CButtons));
    }

    #[test]
    fn skin_files_unpacking_to_too_much_together_are_refused() {
        let big = vec![0; MAX_ENTRY_BYTES];
        let names = [
            "main.bmp",
            "cbuttons.bmp",
            "titlebar.bmp",
            "posbar.bmp",
            "text.bmp",
        ];
        let files: Vec<(&str, &[u8])> = names.iter().map(|n| (*n, big.as_slice())).collect();
        let e = Skin::from_wsz(&zip_of(&files), "bomb").unwrap_err();
        assert!(e.to_string().contains("unpacks to more"), "{e}");
    }

    #[test]
    fn files_too_large_for_a_skin_are_not_read() {
        let e = Skin::from_wsz(&vec![0; MAX_WSZ_BYTES + 1], "big").unwrap_err();
        assert!(e.to_string().contains("too large"), "{e}");
        let path = std::env::temp_dir().join(format!("neoamp-big-{}.wsz", std::process::id()));
        std::fs::write(&path, vec![0; MAX_WSZ_BYTES + 1]).unwrap();
        let e = Skin::from_path(&path).unwrap_err();
        std::fs::remove_file(&path).unwrap();
        assert!(e.to_string().contains("too large"), "{e}");
    }

    #[test]
    fn a_picture_claiming_a_huge_size_is_not_decoded() {
        fn crc32(data: &[u8]) -> u32 {
            let mut c = u32::MAX;
            for b in data {
                c ^= u32::from(*b);
                for _ in 0..8 {
                    c = if c & 1 == 0 {
                        c >> 1
                    } else {
                        0xEDB8_8320 ^ (c >> 1)
                    };
                }
            }
            !c
        }
        fn chunk(png: &mut Vec<u8>, kind: &[u8], data: &[u8]) {
            let body = [kind, data].concat();
            png.extend_from_slice(&u32::try_from(data.len()).unwrap().to_be_bytes());
            png.extend_from_slice(&body);
            png.extend_from_slice(&crc32(&body).to_be_bytes());
        }
        // A PNG saying 100000 x 100000, without the picture.
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        let size = [100_000u32.to_be_bytes(), 100_000u32.to_be_bytes()].concat();
        chunk(&mut png, b"IHDR", &[&size[..], &[8, 6, 0, 0, 0]].concat());
        chunk(&mut png, b"IDAT", &[]);
        chunk(&mut png, b"IEND", &[]);
        let e = decode_image(&png).unwrap_err();
        assert!(e.to_string().contains("unreasonable"), "{e}");
    }
}
