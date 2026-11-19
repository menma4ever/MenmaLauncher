//! Pixel widgets: buttons, the big Play block, the block-by-block update bar,
//! the pack card, the server card and Menma's little mascot.

use super::anim::{approach, breathe, in_out_sine, out_back, out_bounce, out_cubic};
use super::font;
use super::gfx::{bayer, mix, shade, Canvas, Rng, KEY};
use super::sound::{self, Sfx};

pub const INK: u32 = 0xf2efe6;
pub const DIM: u32 = 0x9a96b0;
pub const PANEL: u32 = 0x161427;
pub const BORDER: u32 = 0x3a3560;
pub const GOLD: u32 = 0xf2b33d;
pub const TEAL: u32 = 0x3fc1a5;
pub const RED: u32 = 0xe5534b;
pub const GREEN: u32 = 0x5ad46a;

#[derive(Clone, Copy, Default)]
pub struct Mouse {
    pub x: f32,
    pub y: f32,
    pub inside: bool,
    pub down: bool,
}

impl Mouse {
    pub fn over(&self, x: i32, y: i32, w: i32, h: i32) -> bool {
        self.inside
            && self.x >= x as f32
            && self.y >= y as f32
            && self.x < (x + w) as f32
            && self.y < (y + h) as f32
    }
}

// ---------------- small button ----------------

pub struct Button {
    pub label: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub color: u32,
    hover: f32,
    was_hover: bool,
    flash: f32,
    pub visible: f32,
}

impl Button {
    pub fn new(label: &str, x: i32, y: i32, w: i32, color: u32) -> Button {
        Button { label: label.into(), x, y, w, h: 15, color, hover: 0.0, was_hover: false, flash: 0.0, visible: 1.0 }
    }

    /// Returns true when clicked (mouse released over it).
    pub fn update(&mut self, m: &Mouse, released: bool, dt: f32) -> bool {
        let over = self.visible > 0.5 && m.over(self.x, self.y, self.w, self.h);
        if over && !self.was_hover {
            sound::play(Sfx::Hover);
        }
        self.was_hover = over;
        self.hover = approach(self.hover, if over { 1.0 } else { 0.0 }, 14.0, dt);
        self.flash = approach(self.flash, 0.0, 9.0, dt);
        if over && released {
            self.flash = 1.0;
            sound::play(Sfx::Click);
            return true;
        }
        false
    }

    pub fn draw(&self, c: &mut Canvas, m: &Mouse) {
        if self.visible <= 0.01 {
            return;
        }
        let a = out_cubic(self.visible);
        let pressed = m.down && m.over(self.x, self.y, self.w, self.h);
        let dy = if pressed { 2 } else { 0 };
        let (x, y, w, h) = (self.x, self.y + dy, self.w, self.h);
        // Shadow stays put while the face moves down: that's the "press".
        c.rect_a(self.x + 1, self.y + 3, w, h - 1, 0x05040a, 0.6 * a);
        let body = mix(shade(self.color, -0.45), shade(self.color, -0.25), self.hover);
        c.panel(x, y, w, h, body, mix(shade(self.color, -0.1), self.color, self.hover), a);
        c.rect_a(x + 1, y + h - 2, w - 2, 1, shade(body, -0.3), a);
        let tw = font::width(&self.label, 1);
        c.text_sh(x + (w - tw) / 2, y + 4, &self.label, mix(INK, 0xffffff, self.hover), 1, a);
        if self.flash > 0.02 {
            c.rect_a(x + 1, y + 1, w - 2, h - 2, 0xffffff, self.flash * 0.55);
        }
    }
}

// ---------------- the Play block ----------------

struct Shard {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    px: Vec<u32>,
    size: i32,
}

#[derive(PartialEq, Clone, Copy)]
pub enum PlayState {
    Ready,
    Cracking(f32),
    Broken,
    Reforming(f32),
}

pub struct PlayButton {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub label: String,
    pub enabled: bool,
    hover: f32,
    was_hover: bool,
    pub state: PlayState,
    shards: Vec<Shard>,
    rng: Rng,
    click_at: (f32, f32),
}

impl PlayButton {
    pub fn new(x: i32, y: i32) -> PlayButton {
        PlayButton {
            x,
            y,
            w: 120,
            h: 36,
            label: "PLAY".into(),
            enabled: true,
            hover: 0.0,
            was_hover: false,
            state: PlayState::Ready,
            shards: Vec::new(),
            rng: Rng(0xb10c_b10c),
            click_at: (0.0, 0.0),
        }
    }

    /// True on the frame the block finishes breaking (then the game should start).
    pub fn update(&mut self, m: &Mouse, released: bool, dt: f32, canvas: &Canvas) -> bool {
        let over = m.over(self.x, self.y, self.w, self.h);
        let ready = self.state == PlayState::Ready && self.enabled;
        if over && ready && !self.was_hover {
            sound::play(Sfx::Hover);
        }
        self.was_hover = over && ready;
        self.hover = approach(self.hover, if over && ready { 1.0 } else { 0.0 }, 10.0, dt);
        let mut broke = false;
        match self.state {
            PlayState::Ready => {
                if over && released && self.enabled {
                    sound::play(Sfx::Click);
                    self.click_at = (m.x, m.y);
                    self.state = PlayState::Cracking(0.0);
                }
            }
            PlayState::Cracking(t) => {
                let t = t + dt / 0.16;
                if t >= 1.0 {
                    self.shatter(canvas);
                    sound::play(Sfx::Break);
                    self.state = PlayState::Broken;
                    broke = true;
                } else {
                    self.state = PlayState::Cracking(t);
                }
            }
            PlayState::Reforming(t) => {
                let t = t + dt / 0.5;
                self.state = if t >= 1.0 { PlayState::Ready } else { PlayState::Reforming(t) };
            }
            PlayState::Broken => {}
        }
        for s in &mut self.shards {
            s.vy += 340.0 * dt;
            s.x += s.vx * dt;
            s.y += s.vy * dt;
        }
        self.shards.retain(|s| s.y < 260.0);
        broke
    }

    pub fn reform(&mut self) {
        if self.state == PlayState::Broken {
            self.state = PlayState::Reforming(0.0);
        }
    }

    fn shatter(&mut self, canvas: &Canvas) {
        // The real on-screen pixels of the button fly apart in 6x6 chunks.
        let size = 6;
        let (cx, cy) = self.click_at;
        for by in (0..self.h).step_by(size as usize) {
            for bx in (0..self.w).step_by(size as usize) {
                let mut px = Vec::with_capacity((size * size) as usize);
                for y in 0..size {
                    for x in 0..size {
                        px.push(canvas.get(self.x + bx + x, self.y + by + y));
                    }
                }
                let (sx, sy) = ((self.x + bx) as f32, (self.y + by) as f32);
                let (dx, dy) = (sx - cx, sy - cy);
                let d = (dx * dx + dy * dy).sqrt().max(4.0);
                let force = self.rng.range(60.0, 150.0);
                self.shards.push(Shard {
                    x: sx,
                    y: sy,
                    vx: dx / d * force + self.rng.range(-20.0, 20.0),
                    vy: dy / d * force - self.rng.range(60.0, 130.0),
                    px,
                    size,
                });
            }
        }
    }

    pub fn draw(&self, c: &mut Canvas, m: &Mouse, t: f32) {
        for s in &self.shards {
            for y in 0..s.size {
                for x in 0..s.size {
                    c.put(s.x as i32 + x, s.y as i32 + y, s.px[(y * s.size + x) as usize]);
                }
            }
        }
        if self.state == PlayState::Broken {
            return;
        }
        let reforming = matches!(self.state, PlayState::Reforming(_));

        let pressed = m.down && m.over(self.x, self.y, self.w, self.h) && self.state == PlayState::Ready;
        let dy = if pressed { 2 } else { 0 };
        let (x, y, w, h) = (self.x, self.y + dy, self.w, self.h);
        let b = breathe(t, 3.2);
        let base = if self.enabled { GOLD } else { 0x6a6580 };

        // Hover glow: soft pixel rings.
        let glow = self.hover * (0.55 + 0.45 * b);
        if glow > 0.01 && !reforming {
            for r in 1..=5 {
                c.frame(x - r, y - r, w + 2 * r, h + 2 * r, base, glow * 0.16 * (6 - r) as f32 / 5.0);
            }
        }
        let mut layer = Canvas::sized(w, h + 4);
        layer.px.fill(KEY);
        let body = shade(base, -0.08 + 0.07 * b + 0.08 * self.hover);
        layer.rect(0, 3, w, h, 0x241a08); // drop shadow
        layer.rect(1, 0, w - 2, h, 0x2b1d07); // outline
        layer.rect(0, 1, w, h - 2, 0x2b1d07);
        layer.rect(1, 1, w - 2, h - 2, body);
        layer.rect(1, 1, w - 2, (h - 2) / 2, shade(body, 0.1));
        layer.rect(2, 1, w - 4, 1, shade(body, 0.35));
        layer.rect(1, 2, 1, h - 4, shade(body, 0.2));
        layer.rect(1, h - 3, w - 2, 2, shade(body, -0.25));
        layer.rect(w - 2, 2, 1, h - 4, shade(body, -0.15));
        // Pixel speckles like a block face.
        let mut r = Rng(42);
        for _ in 0..26 {
            let (sx, sy) = (2 + (r.next() % (w as u64 - 4)) as i32, 3 + (r.next() % (h as u64 - 7)) as i32);
            layer.blend(sx, sy, shade(body, if r.f() > 0.5 { 0.14 } else { -0.12 }), 0.8);
        }
        let ink = if self.enabled { 0x2b1d07 } else { 0x2a2838 };
        let tw = font::width(&self.label, 3);
        layer.text((w - tw) / 2 + 1, (h - 21) / 2 + 1, &self.label, shade(body, 0.4), 3);
        layer.text((w - tw) / 2, (h - 21) / 2, &self.label, ink, 3);

        if let PlayState::Cracking(k) = self.state {
            // Cracks spread from the click point before it breaks.
            let (cx, cy) = (self.click_at.0 as i32 - x, self.click_at.1 as i32 - y);
            let mut r = Rng(7);
            for _ in 0..6 {
                let (mut px, mut py) = (cx, cy);
                let len = (k * 26.0) as i32;
                let dir = r.range(0.0, std::f32::consts::TAU);
                for i in 0..len {
                    px += (dir.cos() * 1.4).round() as i32 + if i % 3 == 0 { (r.next() % 3) as i32 - 1 } else { 0 };
                    py += (dir.sin() * 1.4).round() as i32;
                    layer.blend(px, py, 0x1a1205, 0.9);
                }
            }
        }

        layer.blit_to(c, x, y, 1.0, |_| 0);
    }

    /// For the reform dissolve the caller passes the background to restore.
    pub fn dissolve_mask(&self, c: &mut Canvas, before: &Canvas) {
        if let PlayState::Reforming(r) = self.state {
            let reveal = in_out_sine(r);
            for yy in -6..self.h + 8 {
                for xx in -6..self.w + 6 {
                    if bayer(xx / 2, yy / 2) > reveal {
                        let (px, py) = (self.x + xx, self.y + yy);
                        c.put(px, py, before.get(px, py));
                    }
                }
            }
        }
    }
}

// ---------------- update bar ----------------

pub struct UpdateBar {
    pub x: i32,
    pub y: i32,
    pub blocks: usize,
    landed: Vec<Option<f32>>, // time each block started dropping
    pub target: f32,          // 0..1
    shown_pct: f32,
    pub alpha: f32,
    last_sound: f32,
    /// Waiting with nothing landed yet: a light sweeps across the empty slots.
    pub busy: bool,
}

impl UpdateBar {
    pub fn new(x: i32, y: i32) -> UpdateBar {
        UpdateBar {
            x,
            y,
            blocks: 20,
            landed: vec![None; 20],
            target: 0.0,
            shown_pct: 0.0,
            alpha: 0.0,
            last_sound: -1.0,
            busy: false,
        }
    }

    pub fn reset(&mut self) {
        self.landed.iter_mut().for_each(|b| *b = None);
        self.target = 0.0;
        self.shown_pct = 0.0;
    }

    pub fn update(&mut self, t: f32, dt: f32, show: bool) {
        self.alpha = approach(self.alpha, if show { 1.0 } else { 0.0 }, 8.0, dt);
        self.shown_pct = approach(self.shown_pct, self.target * 100.0, 6.0, dt);
        // Blocks drop in one at a time, a little after each other, never all at once.
        let want = ((self.target * self.blocks as f32).floor() as usize).min(self.blocks);
        let last = self.landed.iter().filter_map(|b| *b).fold(f32::MIN, f32::max);
        if let Some(i) = self.landed.iter().position(|b| b.is_none())
            && i < want && t - last > 0.045 {
                self.landed[i] = Some(t);
                if t - self.last_sound > 0.06 {
                    sound::play(Sfx::Block);
                    self.last_sound = t;
                }
            }
    }

    pub fn full(&self) -> bool {
        self.landed.iter().all(|b| b.is_some())
    }

    /// Full, and the last block has finished bouncing.
    pub fn settled(&self, t: f32) -> bool {
        self.full() && self.landed.iter().flatten().all(|s| t - s > 0.4) && self.shown_pct > 99.5
    }

    pub fn draw(&self, c: &mut Canvas, t: f32) {
        if self.alpha < 0.01 {
            return;
        }
        let a = self.alpha;
        let (bw, gap) = (9, 1);
        let w = self.blocks as i32 * (bw + gap) + 3;
        c.panel(self.x - 2, self.y - 2, w, 15, 0x0d0c18, BORDER, a);
        for (i, b) in self.landed.iter().enumerate() {
            let bx = self.x + i as i32 * (bw + gap);
            // Empty slot (with a soft sweep while we wait for the first block)
            c.rect_a(bx, self.y, bw, 11, 0x1c1a30, a * 0.8);
            if self.busy && b.is_none() {
                let sweep = (t * 0.9).fract() * 1.6 - 0.3;
                let d = (i as f32 / self.blocks as f32 - sweep).abs();
                let glow = (1.0 - d * 6.0).max(0.0);
                if glow > 0.0 {
                    c.rect_a(bx, self.y, bw, 11, 0x3a3866, a * glow * in_out_sine(glow));
                }
            }
            if let Some(start) = b {
                let p = ((t - start) / 0.38).clamp(0.0, 1.0);
                let off = ((1.0 - out_bounce(p)) * -16.0) as i32;
                let col = mix(TEAL, GOLD, i as f32 / (self.blocks - 1) as f32);
                let fade = out_cubic(p * 2.0);
                c.rect_a(bx, self.y + off, bw, 11, col, a * fade);
                c.rect_a(bx, self.y + off, bw, 2, shade(col, 0.3), a * fade);
                c.rect_a(bx, self.y + off + 9, bw, 2, shade(col, -0.3), a * fade);
                c.blend(bx + 2, self.y + off + 3, shade(col, 0.45), a * fade);
            }
        }
        let pct = format!("{}%", self.shown_pct.round() as i32);
        c.text_sh(self.x + w + 4, self.y + 2, &pct, INK, 1, a);
    }
}

// ---------------- pack card (tilts toward the mouse) ----------------

const CARD_W: i32 = 134;

pub struct PackCard {
    pub x: i32,
    pub y: i32,
    tilt: f32,
}

impl PackCard {
    pub fn new(x: i32, y: i32) -> PackCard {
        PackCard { x, y, tilt: 0.0 }
    }

    pub fn update(&mut self, m: &Mouse, dt: f32) {
        let (w, h) = (CARD_W, 44);
        let target = if m.over(self.x, self.y, w, h) {
            ((m.x - (self.x + w / 2) as f32) / (w as f32 / 2.0)).clamp(-1.0, 1.0)
        } else {
            0.0
        };
        self.tilt = approach(self.tilt, target, 9.0, dt);
    }

    pub fn draw(&self, c: &mut Canvas, title: &str, line1: &str, line2: &str, a: f32) {
        let (w, h) = (CARD_W, 44);
        let mut card = Canvas::sized(w, h);
        card.px.fill(KEY);
        card.panel(0, 0, w, h, PANEL, mix(BORDER, TEAL, self.tilt.abs() * 0.8), 1.0);
        // Pack icon: a little pixel cube in our colors.
        let (ix, iy) = (6, 8);
        card.rect(ix, iy, 26, 26, 0x0a0914);
        card.rect(ix + 1, iy + 1, 24, 8, shade(TEAL, 0.1));
        card.rect(ix + 1, iy + 9, 24, 16, shade(GOLD, -0.35));
        for k in 0..6 {
            card.rect(ix + 1 + k * 4, iy + 9 + (k % 2) * 2, 2, 2, shade(TEAL, -0.1));
        }
        card.text_sh(38, 8, title, INK, 1, 1.0);
        card.text_sh(38, 20, line1, DIM, 1, 1.0);
        card.text_sh(38, 30, line2, DIM, 1, 1.0);
        // Pixel tilt: shear rows (top and bottom slide opposite ways).
        let k = self.tilt * 4.0;
        card.blit_to(c, self.x, self.y, a, |row| ((row as f32 - h as f32 / 2.0) / (h as f32 / 2.0) * k).round() as i32);
    }
}

// ---------------- server card ----------------

pub enum ServerView {
    NotSetUp,
    Checking,
    Offline,
    Online { ping: u32, online: u32, max: u32 },
}

pub struct Head {
    pub name: String,
    pub uuid: String,
    pub img: [u32; 64],
    pub born: f32,
    pub leaving: Option<f32>,
}

pub fn default_head() -> [u32; 64] {
    // Plain original face for players whose skin can't be loaded.
    let mut h = [0xff_8a6a4au32; 64];
    for x in 0..8 {
        h[x] = 0xff_3a2a1e;
        h[8 + x] = 0xff_3a2a1e;
    }
    h[8 * 4 + 1] = 0xff_ffffff;
    h[8 * 4 + 2] = 0xff_2b2b66;
    h[8 * 4 + 5] = 0xff_2b2b66;
    h[8 * 4 + 6] = 0xff_ffffff;
    h[8 * 6 + 3] = 0xff_5a3a2a;
    h[8 * 6 + 4] = 0xff_5a3a2a;
    h
}

pub fn draw_server_card(c: &mut Canvas, x: i32, y: i32, view: &ServerView, heads: &[Head], t: f32, addr: &str) {
    let (w, h) = (126, 70);
    c.panel(x, y, w, h, PANEL, BORDER, 0.92);
    c.text_sh(x + 6, y + 6, "SERVER", INK, 1, 1.0);
    let (dot, label) = match view {
        ServerView::NotSetUp => (0x6a6580, "NOT SET UP".to_string()),
        ServerView::Checking => (GOLD, "CHECKING...".to_string()),
        ServerView::Offline => (RED, "OFFLINE".to_string()),
        ServerView::Online { online, max, .. } => (GREEN, format!("ONLINE {online}/{max}")),
    };
    // Status dot with a pulse ring that expands and fades.
    let (dx, dy) = (x + w - 12, y + 9);
    let pulse = (t % 1.6) / 1.6;
    if !matches!(view, ServerView::NotSetUp) {
        let r = 2 + (out_cubic(pulse) * 5.0) as i32;
        let a = (1.0 - pulse) * 0.5;
        for yy in -r..=r {
            for xx in -r..=r {
                let d = xx * xx + yy * yy;
                if d <= r * r && d > (r - 1) * (r - 1) {
                    c.blend(dx + xx, dy + yy, dot, a);
                }
            }
        }
    }
    c.rect(dx - 2, dy - 1, 5, 3, dot);
    c.rect(dx - 1, dy - 2, 3, 5, dot);
    c.text_sh(x + 6, y + 17, &label, if matches!(view, ServerView::Online { .. }) { GREEN } else { DIM }, 1, 1.0);

    if let ServerView::Online { ping, .. } = view {
        // Signal bars + ms.
        let bars = match ping { 0..=60 => 4, 61..=120 => 3, 121..=250 => 2, _ => 1 };
        let col = match bars { 4 | 3 => GREEN, 2 => GOLD, _ => RED };
        for i in 0..4 {
            let bh = 2 + i * 2;
            let on = i < bars;
            c.rect(x + 82 + i * 3, y + 24 - bh, 2, bh, if on { col } else { 0x2a2740 });
        }
        c.text_sh(x + 98, y + 17, &format!("{ping}MS"), DIM, 1, 1.0);
    }
    if !addr.is_empty() {
        let mut a = addr.to_string();
        if a.len() > 20 {
            a.truncate(19);
            a.push('.');
        }
        c.text_sh(x + 6, y + h - 12, &a, 0x6f6a8c, 1, 1.0);
    }

    // Faces of who's on right now: they pop in one by one.
    let mut slot = 0;
    for hd in heads.iter().take(6) {
        let born = ((t - hd.born) / 0.35).clamp(0.0, 1.0);
        let gone = hd.leaving.map(|l| 1.0 - ((t - l) / 0.3).clamp(0.0, 1.0)).unwrap_or(1.0);
        let scale_f = out_back(born) * in_out_sine(gone) * 2.0;
        let s = scale_f.round() as i32;
        let (hx, hy) = (x + 6 + slot * 19, y + 30);
        slot += 1;
        if s <= 0 {
            continue;
        }
        let size = 8 * s;
        let (ox, oy) = (hx + (16 - size) / 2, hy + (16 - size) / 2);
        c.rect_a(ox - 1, oy - 1, size + 2, size + 2, 0x05040a, 0.8);
        c.blit8(ox, oy, &hd.img, s, 1.0);
    }
    if let ServerView::Online { online: 0, .. } = view {
        c.text_sh(x + 6, y + 34, "NOBODY ON YET", 0x6f6a8c, 1, 1.0);
    }
}

// ---------------- mascot ----------------

#[derive(Clone, Copy, PartialEq)]
pub enum Mood {
    Idle,
    Cheer,
    Worried,
}

pub struct Mascot {
    pub mood: Mood,
    mood_t: f32,
    next_blink: f32,
    blink_t: f32,
    look: f32,
    look_target: f32,
    next_look: f32,
    rng: Rng,
}

impl Mascot {
    pub fn new() -> Mascot {
        Mascot {
            mood: Mood::Idle,
            mood_t: 0.0,
            next_blink: 2.0,
            blink_t: -1.0,
            look: 0.0,
            look_target: 0.0,
            next_look: 1.5,
            rng: Rng(0x0dd1_7e5),
        }
    }

    pub fn set(&mut self, mood: Mood, t: f32) {
        if self.mood != mood {
            self.mood = mood;
            self.mood_t = t;
        }
    }

    pub fn update(&mut self, t: f32, dt: f32, mouse: &Mouse, my_x: f32) {
        if t > self.next_blink {
            self.blink_t = t;
            self.next_blink = t + self.rng.range(2.2, 5.0);
        }
        if t > self.next_look {
            // Look around now and then, or at the mouse if it's near.
            self.look_target = if mouse.inside && self.rng.f() < 0.6 {
                ((mouse.x - my_x) / 60.0).clamp(-1.0, 1.0)
            } else {
                self.rng.range(-1.0, 1.0)
            };
            self.next_look = t + self.rng.range(1.2, 3.5);
        }
        self.look = approach(self.look, self.look_target, 8.0, dt);
    }

    /// Draws the 16x16 mascot at 2x (32x32 on the canvas). (x, y) is the top-left.
    pub fn draw(&self, c: &mut Canvas, x: i32, y: i32, t: f32) {
        let since = t - self.mood_t;
        let mut oy = ((t * 2.0).sin() * 0.6).round() as i32; // idle bob
        let mut ox = 0;
        let mut squash = 0;
        match self.mood {
            Mood::Cheer => {
                // Hop with squash and stretch, three times, then keep smiling.
                let hop = (since % 0.55) / 0.55;
                if since < 1.65 {
                    oy = -((hop * std::f32::consts::PI).sin() * 6.0).round() as i32;
                    squash = if hop < 0.12 || hop > 0.9 { 1 } else { 0 };
                }
            }
            Mood::Worried => {
                if since < 0.5 {
                    ox = ((since * 40.0).sin() * 1.5).round() as i32; // little shiver
                }
            }
            Mood::Idle => {}
        }
        let p = |c: &mut Canvas, px: i32, py: i32, col: u32| {
            c.rect(x + (px + ox) * 2, y + (py + oy) * 2, 2, 2, col);
        };
        let outline = 0x12312f;
        let body = TEAL;
        let light = 0x8ff0d6;
        let dark = 0x2a8f7c;

        // Shadow on the ground (doesn't hop).
        let sw = if oy < -2 { 8 } else { 12 };
        c.rect_a(x + (16 - sw), y + 31, sw * 2, 2, 0x000000, 0.35);

        // Sprout on top.
        p(c, 8, 1 + squash, 0x5aa844);
        p(c, 7, squash, 0x7ccf5a);
        p(c, 6, squash, 0x7ccf5a);
        p(c, 9, 1 + squash, 0x7ccf5a);
        p(c, 10, 1 + squash, 0x9be36e);
        // Body: rounded block, 14 wide, rows 2..14 (squashed when landing).
        let top = 2 + squash;
        for py in top..15 {
            for px in 1..15 {
                let corner = (py == top || py == 14) && (px == 1 || px == 14);
                if corner {
                    continue;
                }
                let edge = py == top || py == 14 || px == 1 || px == 14
                    || ((py == top + 1 || py == 13) && (px == 1 || px == 14));
                let col = if edge {
                    outline
                } else if py == top + 1 || px == 2 {
                    light
                } else if py >= 12 || px == 13 {
                    dark
                } else {
                    body
                };
                p(c, px, py, col);
            }
        }
        // Feet
        p(c, 4, 15, outline);
        p(c, 5, 15, outline);
        p(c, 10, 15, outline);
        p(c, 11, 15, outline);

        // Eyes
        let blinking = self.blink_t >= 0.0 && t - self.blink_t < 0.12;
        let look = self.look.round() as i32;
        let ey = 6 + squash;
        for ex in [4, 9] {
            if blinking || self.mood == Mood::Cheer && since < 1.65 {
                // closed / happy eyes: ^ ^
                if self.mood == Mood::Cheer {
                    p(c, ex, ey + 1, outline);
                    p(c, ex + 1, ey, outline);
                    p(c, ex + 2, ey + 1, outline);
                } else {
                    p(c, ex, ey + 1, outline);
                    p(c, ex + 1, ey + 1, outline);
                    p(c, ex + 2, ey + 1, outline);
                }
            } else {
                p(c, ex, ey, 0xffffff);
                p(c, ex + 1, ey, 0xffffff);
                p(c, ex + 2, ey, 0xffffff);
                p(c, ex, ey + 1, 0xffffff);
                p(c, ex + 1, ey + 1, 0xffffff);
                p(c, ex + 2, ey + 1, 0xffffff);
                let pupil_x = ex + 1 + look;
                p(c, pupil_x, ey, 0x1b1b3a);
                p(c, pupil_x, ey + 1, 0x1b1b3a);
            }
        }
        if self.mood == Mood::Worried {
            // Slanted brows and a sweat drop.
            p(c, 4, ey - 2, outline);
            p(c, 5, ey - 1, outline);
            p(c, 11, ey - 2, outline);
            p(c, 10, ey - 1, outline);
            let drip = ((since * 1.5) % 1.0 * 3.0) as i32;
            p(c, 14, 4 + drip, 0x7fc8ff);
            p(c, 14, 5 + drip, 0x4a9de0);
        }
        // Blush
        p(c, 3, ey + 3, 0xf08a9a);
        p(c, 12, ey + 3, 0xf08a9a);
        // Mouth
        let my = ey + 4;
        match self.mood {
            Mood::Cheer => {
                p(c, 7, my, outline);
                p(c, 8, my, outline);
                p(c, 7, my + 1, 0xd2556a);
                p(c, 8, my + 1, 0xd2556a);
            }
            Mood::Worried => {
                p(c, 6, my + 1, outline);
                p(c, 7, my, outline);
                p(c, 8, my + 1, outline);
                p(c, 9, my, outline);
            }
            Mood::Idle => {
                p(c, 6, my, outline);
                p(c, 7, my + 1, outline);
                p(c, 8, my + 1, outline);
                p(c, 9, my, outline);
            }
        }
        if self.mood == Mood::Cheer && since < 2.0 {
            // Sparkles around while cheering.
            for i in 0..5 {
                let ang = i as f32 * 1.2566 + since * 2.0;
                let r = 12.0 + out_cubic(since / 0.6) * 8.0;
                let sx = x + 16 + (ang.cos() * r) as i32;
                let sy = y + 14 + (ang.sin() * r * 0.7) as i32;
                let a = 1.0 - (since / 2.0);
                c.blend(sx, sy, GOLD, a);
                c.blend(sx + 1, sy, 0xfff3c0, a * 0.7);
                c.blend(sx - 1, sy, 0xfff3c0, a * 0.7);
                c.blend(sx, sy + 1, 0xfff3c0, a * 0.7);
                c.blend(sx, sy - 1, 0xfff3c0, a * 0.7);
            }
        }
    }
}

/// Pixel arrow cursor, drawn only in recorded demos so viewers can follow the clicks.
pub fn draw_cursor(c: &mut Canvas, x: i32, y: i32, down: bool) {
    const ARROW: [&str; 11] = [
        "X.........",
        "XX........",
        "XWX.......",
        "XWWX......",
        "XWWWX.....",
        "XWWWWX....",
        "XWWWWWX...",
        "XWWWXXXX..",
        "XWXWX.....",
        "XX.XWX....",
        "X...XX....",
    ];
    let fill = if down { 0xc8c4dc } else { 0xffffff };
    for (r, row) in ARROW.iter().enumerate() {
        for (k, ch) in row.chars().enumerate() {
            let col = match ch {
                'X' => 0x0b0a14,
                'W' => fill,
                _ => continue,
            };
            c.put(x + k as i32, y + r as i32, col);
        }
    }
}
