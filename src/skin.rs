//! Classic Winamp skin (.wsz) loading: sprite sheets, viscolor.txt, pledit.txt, region.txt.

use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::Path;

use anyhow::{Context, Result};

use crate::canvas::Image;
use crate::sprites::{Sheet, Sprite};

#[derive(Clone, Debug)]
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

#[derive(Clone, Debug, Default)]
pub struct Regions {
    pub normal: Option<Polygons>,
    pub equalizer: Option<Polygons>,
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
    (Sheet::PlEdit, "pledit"),
];

#[derive(Clone)]
pub struct Skin {
    pub name: String,
    sheets: HashMap<Sheet, Image>,
    empty: Image,
    pub vis_colors: [[u8; 4]; 24],
    pub pledit: PleditColors,
    pub regions: Regions,
}

impl Skin {
    pub fn from_path(path: &Path) -> Result<Skin> {
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "skin".into());
        Self::from_wsz(&bytes, &name)
    }

    pub fn from_wsz(bytes: &[u8], name: &str) -> Result<Skin> {
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).context("skin is not a zip archive")?;
        let mut files: HashMap<String, Vec<u8>> = HashMap::new();
        for i in 0..archive.len() {
            let mut f = match archive.by_index(i) {
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
            let base = full_name.rsplit(['/', '\\']).next().unwrap_or("").to_ascii_lowercase();
            let mut buf = Vec::new();
            if f.read_to_end(&mut buf).is_ok() {
                files.entry(base).or_insert(buf);
            }
        }
        Self::from_files(files, name)
    }

    fn from_files(files: HashMap<String, Vec<u8>>, name: &str) -> Result<Skin> {
        let mut sheets = HashMap::new();
        for (sheet, base) in SHEET_FILES {
            for ext in ["bmp", "png", "BMP"] {
                if let Some(bytes) = files.get(&format!("{base}.{}", ext.to_ascii_lowercase())) {
                    match decode_image(bytes) {
                        Ok(img) => {
                            sheets.insert(*sheet, img);
                            break;
                        }
                        Err(e) => log::warn!("{name}: cannot decode {base}.{ext}: {e}"),
                    }
                }
            }
        }
        if sheets.is_empty() {
            anyhow::bail!("{name}: no skin bitmaps found in archive");
        }
        let text = |key: &str| files.get(key).map(|b| String::from_utf8_lossy(b).into_owned());
        let vis_colors = text("viscolor.txt")
            .map(|t| parse_viscolor(&t))
            .unwrap_or(DEFAULT_VIS_COLORS);
        let pledit = text("pledit.txt").map(|t| parse_pledit(&t)).unwrap_or_default();
        let regions = text("region.txt").map(|t| parse_region(&t)).unwrap_or_default();
        Ok(Skin {
            name: name.to_string(),
            sheets,
            empty: Image::new(0, 0),
            vis_colors,
            pledit,
            regions,
        })
    }

    /// Fill in sheets this skin is missing from `base` (Winamp falls back per file).
    pub fn fill_missing_from(&mut self, base: &Skin) {
        // Skins without balance.bmp reuse the volume slider graphics.
        if !self.sheets.contains_key(&Sheet::Balance)
            && let Some(v) = self.sheets.get(&Sheet::Volume)
        {
            self.sheets.insert(Sheet::Balance, v.clone());
        }
        for (k, v) in &base.sheets {
            if *k == Sheet::NumsEx {
                continue; // optional sheet; absence selects the numbers.bmp path
            }
            self.sheets.entry(*k).or_insert_with(|| v.clone());
        }
    }

    pub fn has(&self, s: Sheet) -> bool {
        self.sheets.contains_key(&s)
    }

    pub fn sheet(&self, s: Sheet) -> &Image {
        self.sheets.get(&s).unwrap_or(&self.empty)
    }

    pub fn blit(&self, dst: &mut Image, s: Sprite, x: i32, y: i32) {
        dst.blit(self.sheet(s.sheet), s.x, s.y, s.w, s.h, x, y);
    }

    pub fn blit_clipped(&self, dst: &mut Image, s: Sprite, x: i32, y: i32, clip: crate::canvas::Rect) {
        dst.blit_clipped(self.sheet(s.sheet), s.x, s.y, s.w, s.h, x, y, clip);
    }
}

fn decode_image(bytes: &[u8]) -> Result<Image> {
    let img = image::load_from_memory(bytes)?.to_rgba8();
    let (w, h) = (img.width() as i32, img.height() as i32);
    let mut data = img.into_raw();
    // Skins are opaque bitmaps; normalise alpha so blits never leak transparency.
    for px in data.chunks_exact_mut(4) {
        px[3] = 255;
    }
    Ok(Image::from_rgba(w, h, data))
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

pub fn parse_viscolor(text: &str) -> [[u8; 4]; 24] {
    let mut colors = DEFAULT_VIS_COLORS;
    let mut n = 0;
    for line in text.lines() {
        if n >= 24 {
            break;
        }
        let s = strip_comment(line);
        if s.is_empty() {
            continue;
        }
        let parts: Vec<u8> = s
            .split([',', ' ', '\t'])
            .filter(|p| !p.is_empty())
            .filter_map(|p| p.trim().parse::<i64>().ok())
            .map(|v| v.clamp(0, 255) as u8)
            .collect();
        if parts.len() >= 3 {
            colors[n] = [parts[0], parts[1], parts[2], 255];
            n += 1;
        }
    }
    colors
}

fn parse_color(s: &str) -> Option<[u8; 4]> {
    let s = s.trim().trim_start_matches('#');
    if s.len() < 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some([r, g, b, 255])
}

pub fn parse_pledit(text: &str) -> PleditColors {
    let mut c = PleditColors::default();
    for line in text.lines() {
        let line = line.trim();
        let Some((k, v)) = line.split_once('=') else { continue };
        let k = k.trim().to_ascii_lowercase();
        let v = v.trim();
        match k.as_str() {
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

pub fn parse_region(text: &str) -> Regions {
    let mut regions = Regions::default();
    let mut section = String::new();
    let mut num_points: Vec<usize> = Vec::new();
    let mut points: Vec<i32> = Vec::new();
    let mut flush = |section: &str, num_points: &mut Vec<usize>, points: &mut Vec<i32>| {
        if points.len() >= 6 {
            let polys = build_polygons(num_points, points);
            match section {
                "normal" => regions.normal.get_or_insert(polys),
                "equalizer" => regions.equalizer.get_or_insert(polys),
                _ => return,
            };
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
        let Some((k, v)) = line.split_once('=') else { continue };
        let ints = v
            .split([',', ' ', '\t'])
            .filter(|p| !p.is_empty())
            .filter_map(|p| p.parse::<i32>().ok());
        match k.trim().to_ascii_lowercase().as_str() {
            "numpoints" => num_points = ints.map(|n| n.max(0) as usize).collect(),
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
    let mut polys = Vec::new();
    if num_points.is_empty() {
        polys.push(pts);
        return polys;
    }
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

/// Rasterise polygons into a visibility mask of `w`x`h` (true = visible).
pub fn region_mask(polys: &Polygons, w: i32, h: i32) -> Vec<bool> {
    let mut mask = vec![false; (w * h) as usize];
    for poly in polys {
        if poly.len() < 3 {
            continue;
        }
        for y in 0..h {
            let cy = y as f32 + 0.5;
            let mut xs: Vec<f32> = Vec::new();
            for i in 0..poly.len() {
                let (x1, y1) = poly[i];
                let (x2, y2) = poly[(i + 1) % poly.len()];
                if (y1 <= cy && y2 > cy) || (y2 <= cy && y1 > cy) {
                    xs.push(x1 + (cy - y1) * (x2 - x1) / (y2 - y1));
                }
            }
            xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
            for pair in xs.chunks_exact(2) {
                let x0 = pair[0].round().max(0.0) as i32;
                let x1 = pair[1].round().min(w as f32) as i32;
                for x in x0..x1 {
                    mask[(y * w + x) as usize] = true;
                }
            }
        }
    }
    mask
}
