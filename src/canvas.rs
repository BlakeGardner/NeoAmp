//! A tiny software framebuffer: RGBA8 images with clipped rectangle blits.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && py >= self.y && px < self.x + self.w && py < self.y + self.h
    }

    pub fn offset(&self, dx: i32, dy: i32) -> Rect {
        Rect::new(self.x + dx, self.y + dy, self.w, self.h)
    }

    pub fn intersect(&self, o: &Rect) -> Rect {
        let x1 = self.x.max(o.x);
        let y1 = self.y.max(o.y);
        let x2 = (self.x + self.w).min(o.x + o.w);
        let y2 = (self.y + self.h).min(o.y + o.h);
        Rect::new(x1, y1, (x2 - x1).max(0), (y2 - y1).max(0))
    }

    pub fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }

    pub fn right(&self) -> i32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }
}

#[derive(Clone, Debug)]
pub struct Image {
    pub w: i32,
    pub h: i32,
    /// RGBA8, row-major, no padding.
    pub data: Vec<u8>,
}

impl Image {
    pub fn new(w: i32, h: i32) -> Self {
        let w = w.max(0);
        let h = h.max(0);
        Self { w, h, data: vec![0; (w * h * 4) as usize] }
    }

    pub fn from_rgba(w: i32, h: i32, data: Vec<u8>) -> Self {
        debug_assert_eq!(data.len(), (w * h * 4) as usize);
        Self { w, h, data }
    }

    pub fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.w, self.h)
    }

    pub fn get(&self, x: i32, y: i32) -> [u8; 4] {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return [0, 0, 0, 0];
        }
        let i = ((y * self.w + x) * 4) as usize;
        [self.data[i], self.data[i + 1], self.data[i + 2], self.data[i + 3]]
    }

    pub fn put(&mut self, x: i32, y: i32, c: [u8; 4]) {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return;
        }
        let i = ((y * self.w + x) * 4) as usize;
        self.data[i..i + 4].copy_from_slice(&c);
    }

    pub fn clear(&mut self, c: [u8; 4]) {
        for px in self.data.chunks_exact_mut(4) {
            px.copy_from_slice(&c);
        }
    }

    pub fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: [u8; 4]) {
        let r = Rect::new(x, y, w, h).intersect(&self.bounds());
        if r.is_empty() {
            return;
        }
        for row in r.y..r.bottom() {
            let start = ((row * self.w + r.x) * 4) as usize;
            let end = start + (r.w * 4) as usize;
            for px in self.data[start..end].chunks_exact_mut(4) {
                px.copy_from_slice(&c);
            }
        }
    }

    pub fn fill_rect_clipped(&mut self, r: Rect, c: [u8; 4], clip: Rect) {
        let r = r.intersect(&clip);
        self.fill_rect(r.x, r.y, r.w, r.h, c);
    }

    /// Copy a `w`x`h` block from `src` at (`sx`,`sy`) to (`dx`,`dy`), clipped to both images.
    #[allow(clippy::too_many_arguments)]
    pub fn blit(&mut self, src: &Image, sx: i32, sy: i32, w: i32, h: i32, dx: i32, dy: i32) {
        let clip = self.bounds();
        self.blit_clipped(src, sx, sy, w, h, dx, dy, clip);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn blit_clipped(
        &mut self,
        src: &Image,
        sx: i32,
        sy: i32,
        w: i32,
        h: i32,
        dx: i32,
        dy: i32,
        clip: Rect,
    ) {
        let dest = Rect::new(dx, dy, w, h).intersect(&clip).intersect(&self.bounds());
        if dest.is_empty() {
            return;
        }
        // Source rectangle after destination clipping.
        let s = Rect::new(sx + (dest.x - dx), sy + (dest.y - dy), dest.w, dest.h);
        let sc = s.intersect(&src.bounds());
        if sc.is_empty() {
            return;
        }
        let ddx = dest.x + (sc.x - s.x);
        let ddy = dest.y + (sc.y - s.y);
        let n = (sc.w * 4) as usize;
        for row in 0..sc.h {
            let si = (((sc.y + row) * src.w + sc.x) * 4) as usize;
            let di = (((ddy + row) * self.w + ddx) * 4) as usize;
            self.data[di..di + n].copy_from_slice(&src.data[si..si + n]);
        }
    }

    /// Tile `src` block horizontally across `dest` (clipped to it).
    #[allow(clippy::too_many_arguments)]
    pub fn tile_h(&mut self, src: &Image, sx: i32, sy: i32, w: i32, h: i32, dest: Rect) {
        if w <= 0 {
            return;
        }
        let mut x = dest.x;
        while x < dest.right() {
            self.blit_clipped(src, sx, sy, w, h, x, dest.y, dest);
            x += w;
        }
    }

    /// Tile `src` block vertically down `dest` (clipped to it).
    #[allow(clippy::too_many_arguments)]
    pub fn tile_v(&mut self, src: &Image, sx: i32, sy: i32, w: i32, h: i32, dest: Rect) {
        if h <= 0 {
            return;
        }
        let mut y = dest.y;
        while y < dest.bottom() {
            self.blit_clipped(src, sx, sy, w, h, dest.x, y, dest);
            y += h;
        }
    }
}
