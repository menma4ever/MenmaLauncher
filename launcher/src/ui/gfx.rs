//! Low-resolution pixel canvas. Everything is drawn at a small fixed size and then
//! scaled up by a whole number, so every pixel stays sharp at any window size.

use super::font;

pub const W: i32 = 400;
pub const H: i32 = 240;

pub struct Canvas {
    pub w: i32,
    pub h: i32,
    pub px: Vec<u32>,
}

/// Color that means "transparent" in small offscreen canvases.
pub const KEY: u32 = 0xff00ff;

#[inline]
pub fn rgb(r: u8, g: u8, b: u8) -> u32 {
    ((r as u32) << 16) | ((g as u32) << 8) | b as u32
}

#[inline]
pub fn mix(a: u32, b: u32, t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    let ch = |s: u32| {
        let x = ((a >> s) & 0xff) as f32;
        let y = ((b >> s) & 0xff) as f32;
        ((x + (y - x) * t).round() as u32) << s
    };
    ch(16) | ch(8) | ch(0)
}

/// Brighten (t>0) or darken (t<0) a color.
pub fn shade(c: u32, t: f32) -> u32 {
    if t >= 0.0 { mix(c, 0xffffff, t) } else { mix(c, 0x000000, -t) }
}

impl Canvas {
    pub fn new() -> Canvas {
        Canvas::sized(W, H)
    }

    pub fn sized(w: i32, h: i32) -> Canvas {
        Canvas { w, h, px: vec![0; (w * h) as usize] }
    }

    /// Copy this (small, KEY-transparent) canvas onto `dst`, shifting each row by
    /// `row_dx(row)`: used for the pixel tilt of cards. `a` fades the whole sprite.
    pub fn blit_to(&self, dst: &mut Canvas, x: i32, y: i32, a: f32, row_dx: impl Fn(i32) -> i32) {
        for yy in 0..self.h {
            let dx = row_dx(yy);
            for xx in 0..self.w {
                let c = self.px[(yy * self.w + xx) as usize];
                if c != KEY {
                    dst.blend(x + xx + dx, y + yy, c, a);
                }
            }
        }
    }

    #[inline]
    pub fn put(&mut self, x: i32, y: i32, c: u32) {
        if x >= 0 && y >= 0 && x < self.w && y < self.h {
            self.px[(y * self.w + x) as usize] = c;
        }
    }

    #[inline]
    pub fn blend(&mut self, x: i32, y: i32, c: u32, a: f32) {
        if a <= 0.0 || x < 0 || y < 0 || x >= self.w || y >= self.h {
            return;
        }
        let i = (y * self.w + x) as usize;
        self.px[i] = if a >= 1.0 { c } else { mix(self.px[i], c, a) };
    }

    #[inline]
    pub fn get(&self, x: i32, y: i32) -> u32 {
        if x >= 0 && y >= 0 && x < self.w && y < self.h { self.px[(y * self.w + x) as usize] } else { 0 }
    }

    pub fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: u32) {
        self.rect_a(x, y, w, h, c, 1.0);
    }

    pub fn rect_a(&mut self, x: i32, y: i32, w: i32, h: i32, c: u32, a: f32) {
        for yy in y.max(0)..(y + h).min(self.h) {
            for xx in x.max(0)..(x + w).min(self.w) {
                self.blend(xx, yy, c, a);
            }
        }
    }

    pub fn frame(&mut self, x: i32, y: i32, w: i32, h: i32, c: u32, a: f32) {
        self.rect_a(x, y, w, 1, c, a);
        self.rect_a(x, y + h - 1, w, 1, c, a);
        self.rect_a(x, y + 1, 1, h - 2, c, a);
        self.rect_a(x + w - 1, y + 1, 1, h - 2, c, a);
    }

    /// Pixel panel: dark body, 1px border with notched corners, top highlight.
    pub fn panel(&mut self, x: i32, y: i32, w: i32, h: i32, body: u32, border: u32, a: f32) {
        self.rect_a(x + 1, y + 1, w - 2, h - 2, body, a);
        self.rect_a(x + 1, y, w - 2, 1, border, 1.0);
        self.rect_a(x + 1, y + h - 1, w - 2, 1, border, 1.0);
        self.rect_a(x, y + 1, 1, h - 2, border, 1.0);
        self.rect_a(x + w - 1, y + 1, 1, h - 2, border, 1.0);
        self.rect_a(x + 2, y + 1, w - 4, 1, shade(body, 0.12), a);
    }

    pub fn text(&mut self, x: i32, y: i32, s: &str, c: u32, scale: i32) {
        self.text_a(x, y, s, c, scale, 1.0);
    }

    pub fn text_a(&mut self, x: i32, y: i32, s: &str, c: u32, scale: i32, a: f32) {
        let mut cx = x;
        for ch in s.chars() {
            let rows = font::glyph(ch);
            for (ry, row) in rows.iter().enumerate() {
                for rx in 0..5 {
                    if row & (0b10000 >> rx) != 0 {
                        for sy in 0..scale {
                            for sx in 0..scale {
                                self.blend(cx + rx * scale + sx, y + ry as i32 * scale + sy, c, a);
                            }
                        }
                    }
                }
            }
            cx += font::ADVANCE * scale;
        }
    }

    /// Text with a 1px drop shadow, the classic readable pixel look.
    pub fn text_sh(&mut self, x: i32, y: i32, s: &str, c: u32, scale: i32, a: f32) {
        self.text_a(x + scale.max(1), y + scale.max(1), s, 0x0b0a14, scale, a * 0.85);
        self.text_a(x, y, s, c, scale, a);
    }

    pub fn text_center(&mut self, cx: i32, y: i32, s: &str, c: u32, scale: i32, a: f32) {
        let w = font::width(s, scale);
        self.text_sh(cx - w / 2, y, s, c, scale, a);
    }

    /// Draw 8x8 ARGB pixels (skin faces) scaled by `s`.
    pub fn blit8(&mut self, x: i32, y: i32, img: &[u32; 64], s: i32, a: f32) {
        for py in 0..8 {
            for px in 0..8 {
                let v = img[(py * 8 + px) as usize];
                let al = ((v >> 24) & 0xff) as f32 / 255.0;
                if al > 0.0 {
                    self.rect_a(x + px * s, y + py * s, s, s, v & 0xffffff, al * a);
                }
            }
        }
    }

    /// Scale into the window buffer: largest whole-number scale that fits, centered,
    /// letterbox filled with `bg`. Returns (scale, offset_x, offset_y) for mouse mapping.
    pub fn present(&self, out: &mut [u32], ow: u32, oh: u32, bg: u32) -> (i32, i32, i32) {
        let (w, h) = (self.w, self.h);
        let s = ((ow as i32 / w).min(oh as i32 / h)).max(1);
        let ox = (ow as i32 - w * s) / 2;
        let oy = (oh as i32 - h * s) / 2;
        out.fill(bg);
        for y in 0..h {
            let src = &self.px[(y * w) as usize..((y + 1) * w) as usize];
            for sy in 0..s {
                let ty = oy + y * s + sy;
                if ty < 0 || ty >= oh as i32 {
                    continue;
                }
                let row = (ty as u32 * ow) as usize;
                for (x, &c) in src.iter().enumerate() {
                    let tx = ox + x as i32 * s;
                    for sx in 0..s {
                        let xx = tx + sx;
                        if xx >= 0 && xx < ow as i32 {
                            out[row + xx as usize] = c;
                        }
                    }
                }
            }
        }
        (s, ox, oy)
    }
}

/// Tiny deterministic random numbers (xorshift) for particles and generated art.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    pub fn f(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
    pub fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.f()
    }
}

/// 4x4 ordered-dither threshold (0..1) used by the pixel dissolve transition.
pub fn bayer(x: i32, y: i32) -> f32 {
    const M: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
    (M[(y & 3) as usize][(x & 3) as usize] as f32 + 0.5) / 16.0
}
