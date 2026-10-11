//! A tiny software framebuffer: RGBA8 images with clipped rectangle blits.

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
}
