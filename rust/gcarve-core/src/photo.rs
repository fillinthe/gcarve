//! Photo V-Carve picture: an 8-bit grayscale raster kept in the toolpath as base64 (the app decodes PNG/JPEG,
//! the core never touches image formats), sampled bilinearly by `cam::generate_photo`.

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn b64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            out.push(if i <= c.len() { B64[(n >> (18 - 6 * i)) as usize & 63] as char } else { '=' });
        }
    }
    out
}

pub fn b64_decode(s: &str) -> Vec<u8> {
    let val = |c: u8| B64.iter().position(|&b| b == c).map(|p| p as u32);
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0);
    for c in s.bytes().filter_map(val) {
        acc = acc << 6 | c;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

/// Grayscale picture: 0 = black, 255 = white, row 0 at the top
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Gray {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

impl Gray {
    pub fn from_op(w: u32, h: u32, b64: &str) -> Option<Gray> {
        let px = b64_decode(b64);
        (w > 0 && h > 0 && px.len() as u64 == u64::from(w) * u64::from(h)).then_some(Gray { w: w as usize, h: h as usize, px })
    }

    /// Auto levels: the picture's own darkness range (2nd to 98th percentile), stretched to 0..1 by `dark`
    pub fn levels(&self) -> (f64, f64) {
        let mut hist = [0usize; 256];
        self.px.iter().for_each(|&p| hist[p as usize] += 1);
        let n = self.px.len().max(1);
        let at = |frac: f64| {
            let mut acc = 0;
            (0..256).find(|&g| {
                acc += hist[g];
                acc as f64 >= frac * n as f64
            }).unwrap_or(255) as f64
        };
        let (light, dark) = (at(0.98), at(0.02)); // gray values: high = light
        let (lo, hi) = (1.0 - light / 255.0, 1.0 - dark / 255.0);
        if hi - lo < 0.05 { (0.0, 1.0) } else { (lo, hi) } // flat picture: leave it
    }

    /// darkness 0..1 at (u, v) in 0..1 (v = 0 at the bottom, like the document)
    pub fn dark(&self, u: f64, v: f64) -> f64 {
        let x = (u.clamp(0.0, 1.0) * (self.w - 1) as f64).max(0.0);
        let y = ((1.0 - v.clamp(0.0, 1.0)) * (self.h - 1) as f64).max(0.0);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (fx, fy) = (x - x0 as f64, y - y0 as f64);
        let p = |x: usize, y: usize| self.px[y * self.w + x] as f64;
        let g = p(x0, y0) * (1.0 - fx) * (1.0 - fy) + p(x1, y0) * fx * (1.0 - fy) + p(x0, y1) * (1.0 - fx) * fy + p(x1, y1) * fx * fy;
        1.0 - g / 255.0
    }
}

/// A 3D model drawn as a lit picture for Sketch carving (VCarve "Sketch Carving" from a model): light from the
/// upper left, flat tops and the base white (nothing carved), slopes turned away darker. Grey pixels, row 0 at the
/// top, at most `max_px` on the long side.
pub fn shaded_relief(r: &crate::document::Relief, max_px: usize) -> (Vec<u8>, usize, usize) {
    let (nx, ny) = (r.nx.max(0) as usize, r.ny.max(0) as usize);
    if nx < 2 || ny < 2 || r.height.len() < nx * ny {
        return (Vec::new(), 0, 0);
    }
    let k = (nx.max(ny) as f64 / max_px as f64).max(1.0);
    let (w, h) = (((nx as f64 / k) as usize).max(2), ((ny as f64 / k) as usize).max(2));
    let at = |x: f64, y: f64| r.height[(y.round() as usize).min(ny - 1) * nx + (x.round() as usize).min(nx - 1)] as f64;
    let (cell_x, cell_y) = (r.w / nx as f64, r.h / ny as f64);
    let l = { let (x, y, z) = (-1.0f64, 1.0, 1.5); let n = (x * x + y * y + z * z).sqrt(); (x / n, y / n, z / n) };
    let mut px = vec![255u8; w * h];
    for row in 0..h {
        for col in 0..w {
            let (x, y) = (col as f64 * k, (h - 1 - row) as f64 * k); // y up in the model
            if at(x, y) <= 1e-3 {
                continue; // the base: not carved
            }
            let dzdx = (at(x + k, y) - at((x - k).max(0.0), y)) * r.depth / (2.0 * k * cell_x);
            let dzdy = (at(x, y + k) - at(x, (y - k).max(0.0))) * r.depth / (2.0 * k * cell_y);
            let n = { let (a, b) = (-dzdx, -dzdy); let m = (a * a + b * b + 1.0).sqrt(); (a / m, b / m, 1.0 / m) };
            let lit = (n.0 * l.0 + n.1 * l.1 + n.2 * l.2).max(0.0) / l.2; // flat = 1
            px[row * w + col] = (lit.min(1.0) * 255.0).round() as u8;
        }
    }
    (px, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shaded_model_is_white_on_flats_and_dark_on_the_slopes_facing_away() {
        // a 4 in square pyramid, 0.5 deep, on a 1 in margin of base
        let n: usize = 120;
        let height = (0..n * n).map(|i| { let (x, y) = ((i % n) as f64 / n as f64 * 6.0 - 3.0, (i / n) as f64 / n as f64 * 6.0 - 3.0); (1.0 - x.abs().max(y.abs()) / 2.0).max(0.0) as f32 }).collect();
        let r = crate::document::Relief { w: 6.0, h: 6.0, depth: 0.5, nx: n as i32, ny: n as i32, height, ..Default::default() };
        let (px, w, h) = shaded_relief(&r, 60);
        assert_eq!((w, h), (60, 60));
        let g = |x: usize, row: usize| px[row * w + x];
        assert_eq!(g(2, 2), 255, "base");
        // the face towards the light (left, x small) is lighter than the right face; the top face (row small = y up)
        // lighter than the bottom one
        assert!(g(18, 30) > g(42, 30) + 40, "{} {}", g(18, 30), g(42, 30));
        assert!(g(30, 18) > g(30, 42) + 40, "{} {}", g(30, 18), g(30, 42));
    }

    #[test]
    fn base64_round_trip() {
        for d in [&b""[..], b"a", b"ab", b"abc", b"abcd", &[0, 255, 7, 128, 3]] {
            assert_eq!(b64_decode(&b64_encode(d)), d);
        }
        assert_eq!(b64_encode(b"Man"), "TWFu");
    }
}
