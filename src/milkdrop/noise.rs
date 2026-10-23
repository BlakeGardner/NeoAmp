//! MilkDrop's noise textures, which shaders sample as `sampler_noise_lq` and the like:
//! random RGBA texels, smoothed by cubic interpolation between every `zoom`th one for the
//! smoother ("higher quality") kinds. After butterchurn's `noise.js`.

/// A small random number generator (xorshift64*), so the textures need nothing else.
#[derive(Debug)]
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        // The top 24 bits, as a fraction.
        ((self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32) / (1u32 << 24) as f32
    }
}

/// Catmull-Rom style interpolation through four values at `t` between the middle two.
fn cubic(y0: f32, y1: f32, y2: f32, y3: f32, t: f32) -> f32 {
    let (t2, t3) = (t * t, t * t * t);
    let a0 = y3 - y2 - y0 + y1;
    let a1 = y0 - y1 - a0;
    let a2 = y2 - y0;
    a0 * t3 + a1 * t2 + a2 * t + y1
}

/// A noise texture `size` texels along each of `dims` axes (2 or 3), RGBA, smoothed so that
/// every `zoom`th texel along each axis is random and the rest interpolated.
#[must_use]
pub fn noise(size: usize, dims: u32, zoom: usize, seed: u64) -> Vec<u8> {
    let mut rng = Rng(seed | 1);
    let count = size.pow(dims);
    // With smoothing, values stay a little away from 0 and 255, since the curve overshoots.
    let range = if zoom > 1 { 216.0 } else { 256.0 };
    let mut data: Vec<u8> = (0..count * 4)
        .map(|_| (128.0 + (rng.next() - 0.5) * range).clamp(0.0, 255.0) as u8)
        .collect();
    if zoom <= 1 {
        return data;
    }
    let stride = |axis: u32| size.pow(axis);
    // One axis at a time: x on the rows whose other coordinates are multiples of zoom, then y
    // on every x of the planes whose z is a multiple, then z everywhere.
    for axis in 0..dims {
        for index in 0..count {
            let coord = |a: u32| (index / stride(a)) % size;
            let c = coord(axis);
            if c % zoom == 0 || (axis + 1..dims).any(|b| coord(b) % zoom != 0) {
                continue;
            }
            let base = c / zoom * zoom + size;
            let t = (c % zoom) as f32 / zoom as f32;
            let at = |k: usize| {
                index - c * stride(axis) + ((base + k * zoom - zoom) % size) * stride(axis)
            };
            for channel in 0..4 {
                let v = |k| f32::from(data[at(k) * 4 + channel]) / 255.0;
                let f = cubic(v(0), v(1), v(2), v(3), t).clamp(0.0, 1.0);
                data[index * 4 + channel] = (f * 255.0) as u8;
            }
        }
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_smoothing() {
        let rough = noise(32, 2, 1, 1);
        assert_eq!(rough.len(), 32 * 32 * 4);
        let smooth = noise(32, 2, 4, 1);
        let vol = noise(16, 3, 4, 2);
        assert_eq!(vol.len(), 16 * 16 * 16 * 4);
        // Neighbouring texels differ less once smoothed.
        let step = |d: &[u8]| -> f64 {
            d.chunks(4)
                .zip(d.chunks(4).skip(1))
                .map(|(a, b)| f64::from(a[0].abs_diff(b[0])))
                .sum::<f64>()
                / (d.len() / 4) as f64
        };
        assert!(
            step(&smooth) < step(&rough) / 2.0,
            "{} {}",
            step(&smooth),
            step(&rough)
        );
        assert!(step(&vol) < step(&rough) / 2.0);
    }

    #[test]
    fn the_same_seed_gives_the_same_noise() {
        assert_eq!(noise(16, 2, 2, 9), noise(16, 2, 2, 9));
        assert_ne!(noise(16, 2, 2, 9), noise(16, 2, 2, 10));
    }
}
