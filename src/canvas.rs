//! A tiny software framebuffer: RGBA8 images with clipped rectangle blits, and canvases that
//! address an image in skin pixels so the same layout can be drawn at any pixel density.

/// An axis-aligned pixel rectangle. `w` and `h` may be zero or negative, meaning empty.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

const fn min(a: i32, b: i32) -> i32 {
    if a < b { a } else { b }
}

const fn max(a: i32, b: i32) -> i32 {
    if a > b { a } else { b }
}

impl Rect {
    #[must_use]
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    #[must_use]
    pub const fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && py >= self.y && px < self.x + self.w && py < self.y + self.h
    }

    #[must_use]
    pub const fn offset(&self, dx: i32, dy: i32) -> Rect {
        Rect::new(self.x + dx, self.y + dy, self.w, self.h)
    }

    #[must_use]
    pub const fn intersect(&self, o: &Rect) -> Rect {
        let x1 = max(self.x, o.x);
        let y1 = max(self.y, o.y);
        let x2 = min(self.x + self.w, o.x + o.w);
        let y2 = min(self.y + self.h, o.y + o.h);
        Rect::new(x1, y1, max(x2 - x1, 0), max(y2 - y1, 0))
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }

    #[must_use]
    pub const fn right(&self) -> i32 {
        self.x + self.w
    }

    #[must_use]
    pub const fn bottom(&self) -> i32 {
        self.y + self.h
    }

    /// Every coordinate multiplied by `k`, saturating rather than overflowing.
    #[must_use]
    pub const fn scaled(&self, k: i32) -> Rect {
        Rect::new(
            self.x.saturating_mul(k),
            self.y.saturating_mul(k),
            self.w.saturating_mul(k),
            self.h.saturating_mul(k),
        )
    }
}

/// An RGBA8 raster, row-major with no padding.
#[derive(Clone, Debug)]
pub struct Image {
    pub w: i32,
    pub h: i32,
    pub data: Vec<u8>,
}

impl Image {
    /// A transparent black image. Negative sizes are treated as zero.
    #[must_use]
    pub fn new(w: i32, h: i32) -> Self {
        let w = w.max(0);
        let h = h.max(0);
        let len = usize::try_from(w).unwrap_or(0) * usize::try_from(h).unwrap_or(0) * 4;
        Self {
            w,
            h,
            data: vec![0; len],
        }
    }

    /// Wrap existing RGBA8 pixels; `None` if the buffer does not match the size.
    #[must_use]
    pub fn from_rgba(w: i32, h: i32, data: Vec<u8>) -> Option<Self> {
        let expected = usize::try_from(w).ok()? * usize::try_from(h).ok()? * 4;
        (data.len() == expected).then_some(Self { w, h, data })
    }

    #[must_use]
    pub const fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.w, self.h)
    }

    /// Byte offset of a pixel inside `data`, or `None` when out of bounds.
    fn index(&self, x: i32, y: i32) -> Option<usize> {
        if !self.bounds().contains(x, y) {
            return None;
        }
        let (x, y, w) = (
            x.unsigned_abs() as usize,
            y.unsigned_abs() as usize,
            self.w.unsigned_abs() as usize,
        );
        Some((y * w + x) * 4)
    }

    #[must_use]
    pub fn get(&self, x: i32, y: i32) -> [u8; 4] {
        match self.index(x, y) {
            Some(i) => [
                self.data[i],
                self.data[i + 1],
                self.data[i + 2],
                self.data[i + 3],
            ],
            None => [0, 0, 0, 0],
        }
    }

    pub fn put(&mut self, x: i32, y: i32, c: [u8; 4]) {
        if let Some(i) = self.index(x, y) {
            self.data[i..i + 4].copy_from_slice(&c);
        }
    }

    pub fn clear(&mut self, c: [u8; 4]) {
        for px in self.data.chunks_exact_mut(4) {
            px.copy_from_slice(&c);
        }
    }

    pub fn fill_rect(&mut self, r: Rect, c: [u8; 4]) {
        let r = r.intersect(&self.bounds());
        if r.is_empty() {
            return;
        }
        let row_len = r.w.unsigned_abs() as usize * 4;
        for row in r.y..r.bottom() {
            let Some(start) = self.index(r.x, row) else {
                continue;
            };
            for px in self.data[start..start + row_len].chunks_exact_mut(4) {
                px.copy_from_slice(&c);
            }
        }
    }

    pub fn fill_rect_clipped(&mut self, r: Rect, c: [u8; 4], clip: Rect) {
        self.fill_rect(r.intersect(&clip), c);
    }

    /// Copy the `src_rect` block of `src` to (`dx`, `dy`), clipped to both images.
    pub fn blit(&mut self, src: &Image, src_rect: Rect, dx: i32, dy: i32) {
        let clip = self.bounds();
        self.blit_clipped(src, src_rect, dx, dy, clip);
    }

    /// Like [`Image::blit`], additionally clipped to `clip` (in destination coordinates).
    pub fn blit_clipped(&mut self, src: &Image, src_rect: Rect, dx: i32, dy: i32, clip: Rect) {
        let dest = Rect::new(dx, dy, src_rect.w, src_rect.h)
            .intersect(&clip)
            .intersect(&self.bounds());
        if dest.is_empty() {
            return;
        }
        // Source rectangle after destination clipping, then clipped to the source image.
        let s = Rect::new(
            src_rect.x + (dest.x - dx),
            src_rect.y + (dest.y - dy),
            dest.w,
            dest.h,
        );
        let sc = s.intersect(&src.bounds());
        if sc.is_empty() {
            return;
        }
        let ddx = dest.x + (sc.x - s.x);
        let ddy = dest.y + (sc.y - s.y);
        let n = sc.w.unsigned_abs() as usize * 4;
        for row in 0..sc.h {
            let (Some(si), Some(di)) = (src.index(sc.x, sc.y + row), self.index(ddx, ddy + row))
            else {
                continue;
            };
            self.data[di..di + n].copy_from_slice(&src.data[si..si + n]);
        }
    }

    /// Repeat `src_rect` horizontally across `dest` (clipped to it).
    pub fn tile_h(&mut self, src: &Image, src_rect: Rect, dest: Rect) {
        if src_rect.w <= 0 {
            return;
        }
        let mut x = dest.x;
        while x < dest.right() {
            self.blit_clipped(src, src_rect, x, dest.y, dest);
            x += src_rect.w;
        }
    }

    /// Repeat `src_rect` vertically down `dest` (clipped to it).
    pub fn tile_v(&mut self, src: &Image, src_rect: Rect, dest: Rect) {
        if src_rect.h <= 0 {
            return;
        }
        let mut y = dest.y;
        while y < dest.bottom() {
            self.blit_clipped(src, src_rect, dest.x, y, dest);
            y += src_rect.h;
        }
    }
}

/// How many image pixels wide and tall each skin pixel is drawn. Skins lay everything out in
/// their own pixels; a denser canvas draws the same layout with finer pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Density(u8);

impl Density {
    /// One image pixel per skin pixel, as classic skins were drawn.
    pub const ONE: Density = Density(1);
    /// The densest canvas NeoAmp draws.
    pub const MAX: Density = Density(8);

    /// `k` image pixels per skin pixel, or `None` outside 1 to [`Density::MAX`].
    #[must_use]
    pub const fn new(k: u8) -> Option<Density> {
        if k >= Self::ONE.0 && k <= Self::MAX.0 {
            Some(Density(k))
        } else {
            None
        }
    }

    /// Image pixels per skin pixel along each axis.
    #[must_use]
    pub const fn get(self) -> i32 {
        self.0 as i32
    }

    /// The least density whose pixels are no larger than the screen's when the skin is shown
    /// at `scale` screen pixels per skin pixel, so the GPU only ever shrinks the canvas a
    /// little. Capped at [`Density::MAX`]; past it the GPU enlarges instead.
    #[must_use]
    pub fn covering(scale: f64) -> Density {
        let k = (scale - SCALE_TOLERANCE).ceil();
        if k.is_nan() || k <= 1.0 {
            Self::ONE
        } else if k >= f64::from(Self::MAX.0) {
            Self::MAX
        } else {
            // A whole number between 2 and MAX - 1, so the cast is exact.
            Density(k as u8)
        }
    }
}

/// How far a scale may sit above a whole number and still count as it, absorbing float noise
/// such as `1.5 * 2.0 * 1.000_000_1`.
const SCALE_TOLERANCE: f64 = 1e-6;

/// An image addressed in skin pixels. Every position, size and clip rectangle is in the skin's
/// own pixels, and each skin pixel covers `density` × `density` pixels of the image
/// underneath. Windows draw into one, and skin sheets are held as one, so a blit between two
/// of the same density copies whole blocks.
#[derive(Clone, Debug)]
pub struct Canvas {
    image: Image,
    density: Density,
}

impl From<Image> for Canvas {
    /// The image as a canvas of [`Density::ONE`]: one image pixel per skin pixel.
    fn from(image: Image) -> Self {
        Self {
            image,
            density: Density::ONE,
        }
    }
}

impl Canvas {
    /// A transparent canvas `w` × `h` skin pixels in size. Negative sizes are treated as zero.
    #[must_use]
    pub fn new(w: i32, h: i32, density: Density) -> Self {
        let k = density.get();
        Self {
            image: Image::new(w.max(0).saturating_mul(k), h.max(0).saturating_mul(k)),
            density,
        }
    }

    /// `image` as a canvas of `density`; `None` unless its size is a whole number of skin
    /// pixels at that density.
    #[must_use]
    pub fn from_image(image: Image, density: Density) -> Option<Self> {
        let k = density.get();
        (image.w % k == 0 && image.h % k == 0).then_some(Self { image, density })
    }

    #[must_use]
    pub const fn density(&self) -> Density {
        self.density
    }

    /// The pixels underneath: `density` times the canvas size along each axis.
    #[must_use]
    pub const fn image(&self) -> &Image {
        &self.image
    }

    /// The pixels underneath, for drawing finer than whole skin pixels.
    pub fn image_mut(&mut self) -> &mut Image {
        &mut self.image
    }

    /// Width and height in skin pixels.
    #[must_use]
    pub const fn size(&self) -> (i32, i32) {
        let k = self.density.get();
        (self.image.w / k, self.image.h / k)
    }

    /// The whole canvas, in skin pixels.
    #[must_use]
    pub const fn bounds(&self) -> Rect {
        let (w, h) = self.size();
        Rect::new(0, 0, w, h)
    }

    /// The colour of skin pixel (`x`, `y`), read from its top-left image pixel; transparent
    /// black when out of bounds.
    #[must_use]
    pub fn get(&self, x: i32, y: i32) -> [u8; 4] {
        let k = self.density.get();
        self.image.get(x.saturating_mul(k), y.saturating_mul(k))
    }

    /// Paint skin pixel (`x`, `y`); ignored when out of bounds.
    pub fn put(&mut self, x: i32, y: i32, c: [u8; 4]) {
        self.fill_rect(Rect::new(x, y, 1, 1), c);
    }

    pub fn clear(&mut self, c: [u8; 4]) {
        self.image.clear(c);
    }

    pub fn fill_rect(&mut self, r: Rect, c: [u8; 4]) {
        self.image.fill_rect(r.scaled(self.density.get()), c);
    }

    pub fn fill_rect_clipped(&mut self, r: Rect, c: [u8; 4], clip: Rect) {
        self.fill_rect(r.intersect(&clip), c);
    }

    /// Copy the `src_rect` block of `src` to (`dx`, `dy`), clipped to both canvases.
    pub fn blit(&mut self, src: &Canvas, src_rect: Rect, dx: i32, dy: i32) {
        let clip = self.bounds();
        self.blit_clipped(src, src_rect, dx, dy, clip);
    }

    /// Like [`Canvas::blit`], additionally clipped to `clip`. A source of another density is
    /// resampled to the nearest pixel.
    pub fn blit_clipped(&mut self, src: &Canvas, src_rect: Rect, dx: i32, dy: i32, clip: Rect) {
        let k = self.density.get();
        if src.density == self.density {
            self.image.blit_clipped(
                &src.image,
                src_rect.scaled(k),
                dx.saturating_mul(k),
                dy.saturating_mul(k),
                clip.scaled(k),
            );
        } else {
            self.blit_resampled(src, src_rect, dx, dy, clip);
        }
    }

    /// [`Canvas::blit_clipped`] between densities: each image pixel of the destination takes
    /// the source image pixel at the same place within the skin pixel block.
    fn blit_resampled(&mut self, src: &Canvas, src_rect: Rect, dx: i32, dy: i32, clip: Rect) {
        let (kd, ks) = (self.density.get(), src.density.get());
        // Clipped to the source first, so every destination pixel has a source pixel.
        let s = src_rect.intersect(&src.bounds());
        let (dx, dy) = (dx + (s.x - src_rect.x), dy + (s.y - src_rect.y));
        let origin = (dx.saturating_mul(kd), dy.saturating_mul(kd));
        let dest = Rect::new(dx, dy, s.w, s.h)
            .scaled(kd)
            .intersect(&clip.scaled(kd))
            .intersect(&self.image.bounds());
        let from = s.scaled(ks);
        let row_len = dest.w.unsigned_abs() as usize * 4;
        // The source row the last destination row came from, and where that row starts.
        let mut previous: Option<(i32, usize)> = None;
        for y in dest.y..dest.bottom() {
            let Some(row) = self.image.index(dest.x, y) else {
                continue;
            };
            let sy = from.y + (y - origin.1) * ks / kd;
            if let Some((above, start)) = previous
                && above == sy
            {
                // Enlarging: the row above came from the same source row.
                self.image.data.copy_within(start..start + row_len, row);
                continue;
            }
            for x in dest.x..dest.right() {
                let sx = from.x + (x - origin.0) * ks / kd;
                let Some(si) = src.image.index(sx, sy) else {
                    continue;
                };
                let di = row + (x - dest.x).unsigned_abs() as usize * 4;
                self.image.data[di..di + 4].copy_from_slice(&src.image.data[si..si + 4]);
            }
            previous = Some((sy, row));
        }
    }

    /// Repeat `src_rect` horizontally across `dest` (clipped to it).
    pub fn tile_h(&mut self, src: &Canvas, src_rect: Rect, dest: Rect) {
        if src_rect.w <= 0 {
            return;
        }
        let mut x = dest.x;
        while x < dest.right() {
            self.blit_clipped(src, src_rect, x, dest.y, dest);
            x += src_rect.w;
        }
    }

    /// Repeat `src_rect` vertically down `dest` (clipped to it).
    pub fn tile_v(&mut self, src: &Canvas, src_rect: Rect, dest: Rect) {
        if src_rect.h <= 0 {
            return;
        }
        let mut y = dest.y;
        while y < dest.bottom() {
            self.blit_clipped(src, src_rect, dest.x, y, dest);
            y += src_rect.h;
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const RED: [u8; 4] = [255, 0, 0, 255];

    #[test]
    fn intersect_clamps_to_overlap_and_never_goes_negative() {
        let a = Rect::new(0, 0, 10, 10);
        assert_eq!(a.intersect(&Rect::new(5, 5, 10, 10)), Rect::new(5, 5, 5, 5));
        assert!(a.intersect(&Rect::new(20, 20, 5, 5)).is_empty());
    }

    #[test]
    fn from_rgba_rejects_mismatched_buffers() {
        assert!(Image::from_rgba(2, 2, vec![0; 16]).is_some());
        assert!(Image::from_rgba(2, 2, vec![0; 15]).is_none());
        assert!(Image::from_rgba(-1, 2, vec![]).is_none());
    }

    #[test]
    fn blit_is_clipped_on_every_edge() {
        let mut dst = Image::new(4, 4);
        let mut src = Image::new(3, 3);
        src.clear(RED);
        // Hanging off the top-left corner: only the overlapping 2x2 lands.
        dst.blit(&src, src.bounds(), -1, -1);
        assert_eq!(dst.get(0, 0), RED);
        assert_eq!(dst.get(1, 1), RED);
        assert_eq!(dst.get(2, 2), [0, 0, 0, 0]);
        // Hanging off the bottom-right corner.
        dst.blit(&src, src.bounds(), 3, 3);
        assert_eq!(dst.get(3, 3), RED);
        assert_eq!(dst.get(2, 3), [0, 0, 0, 0]);
        // A source rectangle larger than the source image copies only what exists.
        let mut dst2 = Image::new(4, 4);
        dst2.blit(&src, Rect::new(1, 1, 10, 10), 0, 0);
        assert_eq!(dst2.get(1, 1), RED);
        assert_eq!(dst2.get(2, 2), [0, 0, 0, 0]);
    }

    #[test]
    fn tiling_covers_exactly_the_destination() {
        let mut dst = Image::new(10, 2);
        let mut src = Image::new(3, 2);
        src.clear(RED);
        dst.tile_h(&src, src.bounds(), Rect::new(1, 0, 7, 2));
        assert_eq!(dst.get(0, 0), [0, 0, 0, 0]);
        assert_eq!(dst.get(1, 0), RED);
        assert_eq!(dst.get(7, 1), RED);
        assert_eq!(dst.get(8, 0), [0, 0, 0, 0]);
    }

    #[test]
    fn out_of_bounds_access_is_ignored() {
        let mut img = Image::new(2, 2);
        img.put(5, 5, RED);
        img.put(-1, 0, RED);
        assert_eq!(img.get(5, 5), [0, 0, 0, 0]);
        assert!(img.data.iter().all(|b| *b == 0));
    }

    const GREEN: [u8; 4] = [0, 255, 0, 255];
    const CLEAR: [u8; 4] = [0, 0, 0, 0];

    fn density(k: u8) -> Density {
        Density::new(k).unwrap()
    }

    /// A `w` × `h` image where every pixel has its own colour.
    fn pattern(w: i32, h: i32) -> Image {
        let mut img = Image::new(w, h);
        for y in 0..h {
            for x in 0..w {
                img.put(x, y, [x as u8, y as u8, 7, 255]);
            }
        }
        img
    }

    /// The image of `img`, each pixel enlarged to a `k` × `k` block.
    fn enlarged(img: &Image, k: i32) -> Image {
        let mut out = Image::new(img.w * k, img.h * k);
        for y in 0..out.h {
            for x in 0..out.w {
                out.put(x, y, img.get(x / k, y / k));
            }
        }
        out
    }

    #[test]
    fn density_accepts_one_to_max_only() {
        assert_eq!(Density::new(0), None);
        assert_eq!(Density::new(1), Some(Density::ONE));
        assert_eq!(Density::new(8), Some(Density::MAX));
        assert_eq!(Density::new(9), None);
        assert_eq!(density(3).get(), 3);
    }

    #[test]
    fn a_canvas_is_measured_in_skin_pixels_at_any_density() {
        let c = Canvas::new(5, 4, density(3));
        assert_eq!(c.size(), (5, 4));
        assert_eq!(c.bounds(), Rect::new(0, 0, 5, 4));
        assert_eq!((c.image().w, c.image().h), (15, 12));
        assert_eq!(Canvas::new(-2, 4, density(2)).size(), (0, 4));
        assert_eq!(Canvas::from(pattern(3, 2)).size(), (3, 2));
        let dense = Canvas::from_image(pattern(6, 4), density(2)).unwrap();
        assert_eq!((dense.size(), dense.density()), ((3, 2), density(2)));
        assert!(
            Canvas::from_image(pattern(5, 4), density(2)).is_none(),
            "half a skin pixel"
        );
    }

    #[test]
    fn at_density_one_a_canvas_draws_exactly_like_its_image() {
        let src = pattern(9, 7);
        let mut img = Image::new(12, 10);
        let mut c = Canvas::from(Image::new(12, 10));
        let sheet = Canvas::from(src.clone());
        let clip = Rect::new(1, 1, 9, 7);
        img.fill_rect(Rect::new(-2, 3, 6, 3), GREEN);
        c.fill_rect(Rect::new(-2, 3, 6, 3), GREEN);
        img.blit_clipped(&src, Rect::new(2, 1, 8, 8), 3, -1, clip);
        c.blit_clipped(&sheet, Rect::new(2, 1, 8, 8), 3, -1, clip);
        img.tile_h(&src, Rect::new(0, 0, 4, 2), Rect::new(0, 8, 11, 2));
        c.tile_h(&sheet, Rect::new(0, 0, 4, 2), Rect::new(0, 8, 11, 2));
        img.tile_v(&src, Rect::new(5, 0, 1, 3), Rect::new(11, 0, 1, 10));
        c.tile_v(&sheet, Rect::new(5, 0, 1, 3), Rect::new(11, 0, 1, 10));
        img.put(0, 0, RED);
        c.put(0, 0, RED);
        assert_eq!(c.image().data, img.data);
        assert_eq!(c.get(4, 2), img.get(4, 2));
    }

    #[test]
    fn a_dense_canvas_paints_and_reads_whole_skin_pixels() {
        let mut c = Canvas::new(4, 3, density(2));
        c.put(1, 2, RED);
        c.fill_rect_clipped(Rect::new(2, 0, 5, 1), GREEN, c.bounds());
        assert_eq!(c.get(1, 2), RED);
        let img = c.image();
        assert_eq!([img.get(2, 4), img.get(3, 5)], [RED, RED]);
        assert_eq!([img.get(1, 4), img.get(4, 4)], [CLEAR, CLEAR]);
        assert_eq!([img.get(4, 0), img.get(7, 1)], [GREEN, GREEN]);
        assert_eq!(img.get(4, 2), CLEAR);
        c.put(4, 0, RED);
        c.put(-1, 0, RED);
        assert_eq!(c.get(4, 0), CLEAR, "out of bounds");
    }

    #[test]
    fn blits_of_equal_density_copy_blocks_clipped_in_skin_pixels() {
        let src = pattern(6, 5);
        let k = 3;
        let sheet = Canvas {
            image: enlarged(&src, k),
            density: density(3),
        };
        let mut c = Canvas::new(8, 8, density(3));
        // Hanging off the left edge and clipped at the bottom.
        c.blit_clipped(&sheet, Rect::new(1, 1, 4, 4), -1, 2, Rect::new(0, 0, 8, 5));
        let mut expected = Image::new(8, 8);
        expected.blit_clipped(&src, Rect::new(1, 1, 4, 4), -1, 2, Rect::new(0, 0, 8, 5));
        assert_eq!(c.image().data, enlarged(&expected, k).data);
    }

    #[test]
    fn blits_between_densities_take_the_nearest_source_pixel() {
        let src = pattern(4, 4);
        // From one pixel per skin pixel to three: each source pixel becomes a 3x3 block.
        let mut dense = Canvas::new(4, 4, density(3));
        dense.blit(&Canvas::from(src.clone()), Rect::new(0, 0, 4, 4), 0, 0);
        assert_eq!(dense.image().data, enlarged(&src, 3).data);
        // And back: the top-left image pixel of each block.
        let mut sparse = Canvas::new(4, 4, Density::ONE);
        sparse.blit_clipped(&dense, Rect::new(1, 0, 3, 4), 1, 1, Rect::new(0, 0, 3, 4));
        assert_eq!(sparse.get(1, 1), src.get(1, 0));
        assert_eq!(sparse.get(2, 3), src.get(2, 2));
        assert_eq!(sparse.get(3, 1), CLEAR, "clipped");
        assert_eq!(sparse.get(0, 0), CLEAR, "outside the destination");
    }

    #[test]
    fn a_blit_between_densities_copies_only_what_the_source_has() {
        let src = Canvas::from(pattern(3, 3));
        let mut dense = Canvas::new(4, 4, density(2));
        dense.clear(GREEN);
        // The source rectangle hangs off the source's top-left corner by one skin pixel.
        dense.blit(&src, Rect::new(-1, -1, 3, 3), 1, 1);
        assert_eq!(dense.get(1, 1), GREEN, "nothing to copy");
        assert_eq!(dense.get(2, 2), src.get(0, 0));
        assert_eq!(dense.get(3, 3), src.get(1, 1));
        assert_eq!(dense.image().get(7, 6), src.get(1, 1));
        assert_eq!(dense.get(3, 1), GREEN);
    }

    #[test]
    fn the_covering_density_rounds_the_scale_up_within_one_to_max() {
        let k = |scale: f64| Density::covering(scale).get();
        assert_eq!([k(0.5), k(1.0), k(1.5), k(2.0), k(2.5)], [1, 1, 2, 2, 3]);
        assert_eq!(k(1.5 * 2.0 * 1.000_000_1), 3, "float noise is not a step");
        assert_eq!(k(2.000_1), 3);
        assert_eq!([k(7.5), k(8.0), k(24.0)], [8, 8, 8]);
        assert_eq!([k(f64::NAN), k(-3.0), k(f64::INFINITY)], [1, 1, 8]);
    }

    #[test]
    fn dense_tiling_covers_exactly_the_destination() {
        let mut c = Canvas::new(10, 2, density(2));
        let mut sheet = Canvas::new(3, 2, density(2));
        sheet.clear(RED);
        c.tile_h(&sheet, sheet.bounds(), Rect::new(1, 0, 7, 2));
        assert_eq!(
            [c.get(0, 0), c.get(1, 0), c.get(7, 1), c.get(8, 0)],
            [CLEAR, RED, RED, CLEAR]
        );
        let img = c.image();
        assert_eq!(
            [img.get(1, 0), img.get(2, 0), img.get(15, 3), img.get(16, 3)],
            [CLEAR, RED, RED, CLEAR]
        );
    }
}
