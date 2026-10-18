//! Skin sheets drawn as SVG: parsed once with usvg, and rasterised with resvg at each density a
//! window draws them at, the first time it does.
//!
//! An SVG sheet takes the place of the bitmap of the same name (`cbuttons.svg` of
//! `cbuttons.bmp`). Its size, in SVG user units, is the sheet's size in skin pixels, so the
//! sprite coordinates of every classic skin apply unchanged. `docs/SVG_SKINS.md` describes the
//! format for skin makers.

use std::sync::{Arc, OnceLock};
use std::time::Instant;

use anyhow::{Result, bail};
use resvg::{tiny_skia, usvg};

use crate::canvas::{Canvas, Density, Image};

/// Largest SVG file read from a skin. Real sheets are a few hundred kilobytes at most.
const MAX_BYTES: usize = 4 << 20;
/// Largest sheet side, in skin pixels. Classic sheets are under 450.
const MAX_SIDE: f32 = 1024.0;
/// Most image pixels one rasterised sheet may have (64 MB of RGBA). A sheet that would be
/// larger at a window's density is rasterised at the densest density that fits instead, and
/// enlarged from there as it is blitted.
const MAX_RASTER_PIXELS: i64 = 1 << 24;
/// How many densities a sheet can be rasterised at.
const DENSITIES: usize = Density::MAX.get() as usize;

/// An SVG sheet. Clones share the parsed drawing and its rasters.
#[derive(Clone)]
pub(crate) struct SvgSheet(Arc<Inner>);

struct Inner {
    tree: usvg::Tree,
    /// The sheet's size in skin pixels: the SVG's own size, rounded.
    size: (i32, i32),
    /// The sheet rasterised at density `i + 1`, once asked for; `None` if that failed.
    rasters: [OnceLock<Option<Canvas>>; DENSITIES],
}

impl std::fmt::Debug for SvgSheet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SvgSheet")
            .field("size", &self.0.size)
            .finish_non_exhaustive()
    }
}

impl SvgSheet {
    /// Parse an SVG file from a skin. Nothing it names outside itself is loaded: no files, and
    /// (resvg being built without them) no fonts or embedded bitmaps either.
    ///
    /// # Errors
    /// Fails if the file is too big, is not SVG, or its size is not between 1 and
    /// [`MAX_SIDE`] skin pixels along each side.
    pub(crate) fn parse(bytes: &[u8]) -> Result<SvgSheet> {
        if bytes.len() > MAX_BYTES {
            bail!("{} bytes, more than the {MAX_BYTES} allowed", bytes.len());
        }
        let options = usvg::Options {
            image_href_resolver: usvg::ImageHrefResolver {
                resolve_string: Box::new(|_, _| None),
                ..usvg::ImageHrefResolver::default()
            },
            ..usvg::Options::default()
        };
        let tree = usvg::Tree::from_data(bytes, &options)?;
        let (w, h) = (tree.size().width(), tree.size().height());
        if !(1.0..=MAX_SIDE).contains(&w) || !(1.0..=MAX_SIDE).contains(&h) {
            bail!("unreasonable sheet size {w}x{h}");
        }
        Ok(SvgSheet(Arc::new(Inner {
            tree,
            // Between 1 and MAX_SIDE, so the casts are exact.
            size: (w.round() as i32, h.round() as i32),
            rasters: std::array::from_fn(|_| OnceLock::new()),
        })))
    }

    /// The sheet rasterised at `density`, or at the densest density below it that stays within
    /// [`MAX_RASTER_PIXELS`]; `None` if rasterising failed.
    #[must_use]
    pub(crate) fn raster(&self, density: Density) -> Option<&Canvas> {
        let k = self.fitting(density);
        let slot = usize::try_from(k.get() - 1)
            .ok()
            .and_then(|i| self.0.rasters.get(i))?;
        slot.get_or_init(|| self.render(k)).as_ref()
    }

    /// `wanted`, or the densest density below it whose raster is no bigger than
    /// [`MAX_RASTER_PIXELS`].
    fn fitting(&self, wanted: Density) -> Density {
        let (w, h) = self.0.size;
        let pixels = |k: i32| i64::from(w * k) * i64::from(h * k);
        let mut k = wanted.get();
        while k > 1 && pixels(k) > MAX_RASTER_PIXELS {
            k -= 1;
        }
        u8::try_from(k)
            .ok()
            .and_then(Density::new)
            .unwrap_or(Density::ONE)
    }

    fn render(&self, density: Density) -> Option<Canvas> {
        let started = Instant::now();
        let k = density.get();
        let (w, h) = (self.0.size.0 * k, self.0.size.1 * k);
        let mut pixmap = tiny_skia::Pixmap::new(u32::try_from(w).ok()?, u32::try_from(h).ok()?)?;
        // Stretched to exactly the rounded size, so the sheet's skin pixels line up with the
        // canvas's.
        let svg = self.0.tree.size();
        let transform =
            tiny_skia::Transform::from_scale(w as f32 / svg.width(), h as f32 / svg.height());
        resvg::render(&self.0.tree, transform, &mut pixmap.as_mut());
        let mut data = pixmap.take();
        // The pixels are premultiplied, so making them opaque lays the drawing over black:
        // sheets are opaque, as bitmap sheets are.
        for px in data.chunks_exact_mut(4) {
            px[3] = 255;
        }
        let canvas = Image::from_rgba(w, h, data).and_then(|img| Canvas::from_image(img, density));
        log::debug!(
            "rasterised an SVG sheet at {w}x{h} (density {k}) in {:.1} ms",
            started.elapsed().as_secs_f64() * 1000.0
        );
        canvas
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn svg(w: u32, h: u32, body: &str) -> Vec<u8> {
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}">{body}</svg>"#
        )
        .into_bytes()
    }

    fn density(k: u8) -> Density {
        Density::new(k).unwrap()
    }

    #[test]
    fn a_sheet_is_as_many_skin_pixels_as_its_svg_is_units() {
        let sheet = SvgSheet::parse(&svg(
            136,
            36,
            r##"<rect width="68" height="36" fill="#f00"/>"##,
        ))
        .unwrap();
        assert_eq!(sheet.0.size, (136, 36));
        let one = sheet.raster(Density::ONE).unwrap();
        assert_eq!((one.size(), one.density()), ((136, 36), Density::ONE));
        assert_eq!(one.get(10, 10), [255, 0, 0, 255]);
        assert_eq!(
            one.get(100, 10),
            [0, 0, 0, 255],
            "unpainted parts are opaque black"
        );
        let three = sheet.raster(density(3)).unwrap();
        assert_eq!((three.image().w, three.image().h), (408, 108));
        assert_eq!(three.image().get(203, 50), [255, 0, 0, 255]);
        assert_eq!(
            three.image().get(204, 50),
            [0, 0, 0, 255],
            "the edge falls on a pixel edge"
        );
    }

    #[test]
    fn denser_rasters_have_detail_finer_than_a_skin_pixel() {
        let triangle = r##"<polygon points="0,0 20,0 0,20" fill="#fff"/>"##;
        let sheet = SvgSheet::parse(&svg(20, 20, triangle)).unwrap();
        let img = sheet.raster(density(4)).unwrap().image();
        // Skin pixel (9, 10) lies across the slanted edge: its 4x4 block is not one colour.
        let block: Vec<[u8; 4]> = (40..44)
            .flat_map(|y| (36..40).map(move |x| (x, y)))
            .map(|(x, y)| img.get(x, y))
            .collect();
        assert!(block.iter().any(|p| *p != block[0]), "{block:?}");
    }

    #[test]
    fn huge_sheets_are_rasterised_less_densely() {
        let sheet = SvgSheet::parse(&svg(1024, 1024, "")).unwrap();
        // 8192 x 8192 would be four times the budget; 4096 x 4096 fits it exactly.
        assert_eq!(sheet.raster(Density::MAX).unwrap().density(), density(4));
        assert_eq!(sheet.raster(density(2)).unwrap().density(), density(2));
    }

    #[test]
    fn bad_svg_files_are_rejected() {
        assert!(SvgSheet::parse(b"not an svg").is_err());
        assert!(SvgSheet::parse(&svg(5000, 10, "")).is_err(), "too big");
        assert!(SvgSheet::parse(&svg(0, 10, "")).is_err(), "empty");
        assert!(
            SvgSheet::parse(&vec![b' '; MAX_BYTES + 1]).is_err(),
            "too long"
        );
    }

    #[test]
    fn files_an_svg_names_are_not_loaded() {
        // A red SVG on disk, which the sheet asks for by its full path.
        let path = std::env::temp_dir().join(format!("neoamp-svg-{}.svg", std::process::id()));
        std::fs::write(
            &path,
            svg(4, 4, r##"<rect width="4" height="4" fill="#f00"/>"##),
        )
        .unwrap();
        let href = path
            .to_string_lossy()
            .replace('&', "&amp;")
            .replace('"', "&quot;");
        let body = format!(r#"<image href="{href}" width="4" height="4"/>"#);
        let sheet = SvgSheet::parse(&svg(4, 4, &body));
        let _ = std::fs::remove_file(&path);
        let raster = sheet.unwrap().raster(Density::ONE).unwrap().get(1, 1);
        assert_eq!(raster, [0, 0, 0, 255]);
    }

    #[test]
    fn text_is_skipped_rather_than_drawn_with_some_font() {
        let sheet =
            SvgSheet::parse(&svg(40, 10, r#"<text y="9" font-size="9">Llama</text>"#)).unwrap();
        let img = sheet.raster(density(2)).unwrap().image();
        assert!(img.data.chunks_exact(4).all(|p| p == [0, 0, 0, 255]));
    }
}
