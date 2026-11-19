//! The living background, three parallax layers:
//! back = slowly turning world panorama, middle = sky with clouds, sun, moon and stars
//! on the real local clock, front = leaves by day and fireflies by night that shy away
//! from the mouse.

use super::anim::{approach, in_out_sine};
use super::gfx::{mix, rgb, shade, Canvas, Rng, H, W};
use std::f32::consts::PI;

const STRIP: i32 = 960; // panorama loop width in pixels
const HORIZON: i32 = 150;

struct Cloud {
    x: f32,
    y: f32,
    speed: f32,
    mask: Vec<Vec<bool>>,
    depth: f32,
}

struct Particle {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    phase: f32,
    firefly: bool,
    color: u32,
}

pub struct Scene {
    far: Vec<i32>,
    mid: Vec<i32>,
    trees: Vec<(i32, i32, i32)>, // x, height, kind
    stars: Vec<(i32, i32, f32, f32)>,
    clouds: Vec<Cloud>,
    parts: Vec<Particle>,
    rng: Rng,
    par_x: f32,
    par_y: f32,
    /// Optional real panorama strip (STRIP x HORIZON+..), replaces generated hills when set.
    pub panorama: Option<(Vec<u32>, i32, i32)>,
}

fn value_noise(seed: u64, x: f32) -> f32 {
    let h = |i: i64| {
        let mut r = Rng(seed ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        r.next();
        r.f()
    };
    let i = x.floor() as i64;
    let f = x - i as f32;
    let a = h(i.rem_euclid(1 << 20));
    let b = h((i + 1).rem_euclid(1 << 20));
    a + (b - a) * in_out_sine(f)
}

/// Heights that wrap seamlessly around the strip.
fn ridge(seed: u64, base: f32, amp: f32, freq: f32) -> Vec<i32> {
    (0..STRIP)
        .map(|x| {
            let u = x as f32 / STRIP as f32;
            // Blend two copies so the end meets the start.
            let n = |s: f32| {
                value_noise(seed, s * freq) * 0.65 + value_noise(seed ^ 77, s * freq * 2.7) * 0.35
            };
            let v = n(u) * (1.0 - u) + n(u - 1.0) * u;
            (base - v * amp) as i32
        })
        .collect()
}

impl Scene {
    pub fn new() -> Scene {
        let mut rng = Rng(0x5eed_1234_abcd);
        let far = ridge(11, HORIZON as f32 - 12.0, 46.0, 9.0);
        let mid = ridge(23, HORIZON as f32 + 18.0, 30.0, 16.0);
        let mut trees = Vec::new();
        let mut x = 0;
        while x < STRIP {
            x += rng.range(7.0, 26.0) as i32;
            trees.push((x, rng.range(7.0, 15.0) as i32, (rng.next() % 3) as i32));
        }
        let stars = (0..80)
            .map(|_| {
                (
                    rng.range(0.0, W as f32) as i32,
                    rng.range(0.0, 110.0) as i32,
                    rng.range(0.6, 2.4),
                    rng.range(0.0, std::f32::consts::TAU),
                )
            })
            .collect();
        let clouds = (0..9)
            .map(|i| {
                let depth = if i % 3 == 0 { 1.0 } else { 0.55 };
                Scene::make_cloud(&mut rng, depth)
            })
            .collect();
        let parts = Vec::new();
        Scene { far, mid, trees, stars, clouds, parts, rng, par_x: 0.0, par_y: 0.0, panorama: None }
    }

    fn make_cloud(rng: &mut Rng, depth: f32) -> Cloud {
        let w = rng.range(26.0, 58.0) as usize;
        let h = rng.range(8.0, 14.0) as usize;
        let mut mask = vec![vec![false; w]; h];
        // A few overlapping blocky puffs, flat bottom.
        for _ in 0..5 {
            let cx = rng.range(4.0, w as f32 - 4.0);
            let r = rng.range(3.0, h as f32 * 0.8);
            for (y, row) in mask.iter_mut().enumerate() {
                for (x, cell) in row.iter_mut().enumerate() {
                    let dx = (x as f32 - cx) / 1.6;
                    let dy = y as f32 - (h as f32 - r);
                    if dx * dx + dy * dy <= r * r && y + 1 < h {
                        *cell = true;
                    }
                }
            }
        }
        for row in mask.iter_mut().skip(h - 2) {
            for x in 2..w.saturating_sub(2) {
                row[x] = true;
            }
        }
        Cloud {
            x: rng.range(-60.0, W as f32),
            y: rng.range(8.0, 90.0),
            speed: rng.range(1.5, 4.0) * depth,
            mask,
            depth,
        }
    }

    /// Local time of day as hours 0..24 (fractional).
    pub fn clock_hours() -> f32 {
        #[cfg(windows)]
        unsafe {
            let mut st: windows_sys::Win32::Foundation::SYSTEMTIME = std::mem::zeroed();
            windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut st);
            st.wHour as f32 + st.wMinute as f32 / 60.0 + st.wSecond as f32 / 3600.0
        }
        #[cfg(not(windows))]
        12.0
    }

    pub fn night_factor(h: f32) -> f32 {
        // 1 at night, 0 by day, eased through dawn (5-8) and dusk (17.5-20.5).
        if (8.0..=17.5).contains(&h) {
            0.0
        } else if h > 17.5 && h < 20.5 {
            in_out_sine((h - 17.5) / 3.0)
        } else if h > 5.0 && h < 8.0 {
            1.0 - in_out_sine((h - 5.0) / 3.0)
        } else {
            1.0
        }
    }

    fn sky(h: f32) -> (u32, u32) {
        const K: [(f32, u32, u32); 9] = [
            (0.0, 0x0b0d24, 0x1a1840),
            (5.0, 0x141638, 0x3a2a5a),
            (6.5, 0x2c3770, 0xc8735a),
            (9.0, 0x2a5f96, 0x77a6c8),
            (15.5, 0x2a5f96, 0x80aecd),
            (18.0, 0x3a3f7a, 0xd8825a),
            (19.5, 0x1b1c45, 0x4b2d5e),
            (21.0, 0x0e1030, 0x22204a),
            (24.0, 0x0b0d24, 0x1a1840),
        ];
        let i = K.iter().rposition(|k| k.0 <= h).unwrap_or(0).min(K.len() - 2);
        let (a, b) = (K[i], K[i + 1]);
        let t = in_out_sine((h - a.0) / (b.0 - a.0));
        (mix(a.1, b.1, t), mix(a.2, b.2, t))
    }

    pub fn update(&mut self, dt: f32, t: f32, mouse: Option<(f32, f32)>, night: f32) {
        // Parallax follows the mouse, eased.
        let (tx, ty) = match mouse {
            Some((mx, my)) => ((mx / W as f32 - 0.5) * 2.0, (my / H as f32 - 0.5) * 2.0),
            None => (0.0, 0.0),
        };
        self.par_x = approach(self.par_x, tx, 3.0, dt);
        self.par_y = approach(self.par_y, ty, 3.0, dt);

        for c in &mut self.clouds {
            c.x += c.speed * dt;
            if c.x > W as f32 + 20.0 {
                c.x = -(c.mask[0].len() as f32) - 20.0;
            }
        }

        // Keep ~26 particles; new ones match the time of day.
        while self.parts.len() < 26 {
            let firefly = self.rng.f() < night;
            let leaf_colors = [0x6fae4a, 0x8cc152, 0xd9a03f, 0xc4642e];
            let color = if firefly {
                0xfff27a
            } else {
                leaf_colors[(self.rng.next() % 4) as usize]
            };
            let (x, y) = if firefly {
                (self.rng.range(0.0, W as f32), self.rng.range(90.0, H as f32 - 10.0))
            } else {
                (self.rng.range(-20.0, W as f32), self.rng.range(-30.0, -4.0))
            };
            let phase = self.rng.range(0.0, std::f32::consts::TAU);
            self.parts.push(Particle { x, y, vx: 0.0, vy: 0.0, phase, firefly, color });
        }
        for p in &mut self.parts {
            if p.firefly {
                p.vx = approach(p.vx, (t * 0.7 + p.phase).sin() * 6.0, 2.0, dt);
                p.vy = approach(p.vy, (t * 0.9 + p.phase * 1.3).cos() * 4.0, 2.0, dt);
            } else {
                p.vx = approach(p.vx, 5.0 + (t * 1.3 + p.phase).sin() * 7.0, 2.0, dt);
                p.vy = approach(p.vy, 9.0, 1.5, dt);
            }
            if let Some((mx, my)) = mouse {
                let (dx, dy) = (p.x - mx, p.y - my);
                let d2 = dx * dx + dy * dy;
                if d2 < 30.0 * 30.0 && d2 > 0.01 {
                    let push = (1.0 - d2.sqrt() / 30.0) * 90.0 * dt;
                    let d = d2.sqrt();
                    p.vx += dx / d * push;
                    p.vy += dy / d * push;
                }
            }
            p.x += p.vx * dt;
            p.y += p.vy * dt;
        }
        let night_now = night;
        let rng = &mut self.rng;
        self.parts.retain(|p| {
            let off = p.y > H as f32 + 6.0 || p.x > W as f32 + 20.0 || p.x < -30.0 || p.y < -40.0;
            // Fireflies fade out by day, leaves stop falling at night: retire gradually.
            let wrong_time = if p.firefly { night_now < 0.2 } else { night_now > 0.8 };
            !(off || (wrong_time && rng.f() < 0.004))
        });
    }

    pub fn draw(&self, c: &mut Canvas, t: f32, hour: f32) {
        let night = Scene::night_factor(hour);
        let (top, bottom) = Scene::sky(hour);
        for y in 0..H {
            let k = (y as f32 / (HORIZON + 10) as f32).min(1.0);
            let col = mix(top, bottom, in_out_sine(k));
            for x in 0..W {
                c.put(x, y, col);
            }
        }

        // Stars
        for &(sx, sy, sp, ph) in &self.stars {
            let a = night * (0.45 + 0.55 * (0.5 + 0.5 * (t * sp + ph).sin()));
            let x = sx + -self.par_x as i32;
            c.blend(x, sy, 0xf3f0ff, a);
            if sp > 2.0 {
                for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    c.blend(x + dx, sy + dy, 0xb8c4ff, a * 0.4);
                }
            }
        }

        // Sun (6h-18h) and moon (18h-6h) on arcs across the sky.
        let arc = |frac: f32| -> (i32, i32) {
            let x = -14.0 + frac * (W as f32 + 28.0);
            let y = HORIZON as f32 - 8.0 - (PI * frac).sin() * 118.0;
            (x as i32, y as i32)
        };
        if (5.5..18.5).contains(&hour) {
            let (x, y) = arc((hour - 6.0) / 12.0);
            Scene::disc(c, x, y, 7, 0xffd76a, 0.9);
            Scene::disc(c, x, y, 5, 0xffeaa8, 1.0);
            for r in 9..14 {
                Scene::ring(c, x, y, r, 0xffd76a, 0.06 * (14 - r) as f32 / 5.0);
            }
        }
        let mh = if hour >= 17.5 { hour - 18.0 } else { hour + 6.0 };
        if (-0.5..12.5).contains(&mh) {
            let (x, y) = arc(mh / 12.0);
            Scene::disc(c, x, y, 6, 0xdfe6ff, 0.95);
            Scene::disc(c, x + 3, y - 2, 5, mix(top, bottom, 0.3), 1.0); // crescent bite
            Scene::ring(c, x, y, 8, 0xdfe6ff, 0.08);
        }

        // Clouds (middle layer)
        let cloud_col = mix(0xf4f2ff, 0x3a3c66, night);
        for cl in &self.clouds {
            let ox = cl.x + self.par_x * -4.0 * cl.depth;
            let oy = cl.y + self.par_y * -2.0 * cl.depth;
            let a = (0.55 + 0.35 * cl.depth) * (1.0 - night * 0.35);
            for (y, row) in cl.mask.iter().enumerate() {
                for (x, &on) in row.iter().enumerate() {
                    if on {
                        let edge_shade = if y + 3 >= cl.mask.len() { -0.12 } else { 0.0 };
                        c.blend(ox as i32 + x as i32, oy as i32 + y as i32, shade(cloud_col, edge_shade), a);
                    }
                }
            }
        }

        // Back layer: the panorama, turning slowly (one loop every ~5 minutes).
        let scroll = (t * STRIP as f32 / 300.0) + self.par_x * -2.0;
        if let Some((img, iw, ih)) = &self.panorama {
            let y0 = HORIZON + 60 - ih;
            for y in 0..*ih {
                for x in 0..W {
                    let sx = ((x as f32 + scroll) as i32).rem_euclid(*iw);
                    let v = img[(y * iw + sx) as usize];
                    c.put(x, y0 + y, mix(v & 0xffffff, 0x10122a, night * 0.6));
                }
            }
        } else {
            let far_col = mix(mix(bottom, 0x4a5a8a, 0.55), 0x1a1b3a, night * 0.6);
            let mid_col = mix(rgb(52, 92, 70), 0x14182e, night * 0.7);
            let near_col = mix(rgb(38, 66, 52), 0x0e1124, night * 0.75);
            for x in 0..W {
                let fx = ((x as f32 + scroll * 0.5) as i32).rem_euclid(STRIP);
                let top_far = self.far[fx as usize] + -self.par_y as i32;
                for y in top_far.max(0)..H {
                    c.put(x, y, if y == top_far { shade(far_col, 0.08) } else { far_col });
                }
                let mx = ((x as f32 + scroll) as i32).rem_euclid(STRIP);
                let top_mid = self.mid[mx as usize] + (self.par_y * -2.0) as i32;
                for y in top_mid.max(0)..H {
                    let col = if y < top_mid + 2 { shade(mid_col, 0.12) } else { mid_col };
                    c.put(x, y, col);
                }
            }
            // Blocky trees standing on the mid ridge.
            for &(tx, th, kind) in &self.trees {
                let x = (tx as f32 - scroll).rem_euclid(STRIP as f32) as i32;
                if x > W + 8 {
                    continue;
                }
                let ground = self.mid[tx.rem_euclid(STRIP) as usize] + (self.par_y * -2.0) as i32;
                let trunk = mix(rgb(84, 58, 40), 0x14121e, night * 0.7);
                c.rect(x, ground - th / 2, 2, th / 2 + 1, trunk);
                let leaf = mix([rgb(60, 120, 64), rgb(48, 104, 70), rgb(92, 128, 56)][kind as usize], 0x111830, night * 0.7);
                let cw = 6 + kind * 2;
                c.rect(x + 1 - cw / 2, ground - th, cw, th / 2 + 1, leaf);
                c.rect(x + 2 - cw / 2, ground - th - 2, cw - 2, 2, shade(leaf, 0.1));
            }
            let _ = near_col;
        }

        // Front layer: particles
        for p in &self.parts {
            let x = (p.x + self.par_x * -8.0) as i32;
            let y = (p.y + self.par_y * -5.0) as i32;
            if p.firefly {
                let pulse = 0.4 + 0.6 * (0.5 + 0.5 * (t * 2.3 + p.phase).sin());
                for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    c.blend(x + dx, y + dy, p.color, 0.25 * pulse);
                }
                c.blend(x, y, 0xffffe0, pulse);
            } else {
                let flip = ((t * 3.0 + p.phase).sin() > 0.0) as i32;
                c.blend(x, y, p.color, 0.95);
                c.blend(x + 1 - flip, y + flip, shade(p.color, -0.2), 0.9);
            }
        }

        // Darken the lower area so panels and text stay readable (dark theme).
        for y in 0..H {
            let k = ((y - 110) as f32 / (H - 110) as f32).clamp(0.0, 1.0);
            if k > 0.0 {
                for x in 0..W {
                    c.blend(x, y, 0x07070f, k * 0.6);
                }
            }
        }
    }

    fn disc(c: &mut Canvas, cx: i32, cy: i32, r: i32, col: u32, a: f32) {
        for y in -r..=r {
            for x in -r..=r {
                if x * x + y * y <= r * r + r / 2 {
                    c.blend(cx + x, cy + y, col, a);
                }
            }
        }
    }

    fn ring(c: &mut Canvas, cx: i32, cy: i32, r: i32, col: u32, a: f32) {
        for y in -r..=r {
            for x in -r..=r {
                let d = x * x + y * y;
                if d <= r * r + r && d > (r - 1) * (r - 1) + (r - 1) {
                    c.blend(cx + x, cy + y, col, a);
                }
            }
        }
    }
}
