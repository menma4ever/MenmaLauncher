//! Scripted demos for recording the animation proof clips. A virtual mouse moves
//! through the real UI code paths; nothing touches the real cursor. Server players
//! in the demo are made up, with generated (original) faces.

use super::anim::in_out_sine;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Play,
    Buttons,
    Card,
    Server,
    Error,
    DayNight,
}

impl Kind {
    pub fn parse(s: &str) -> Option<Kind> {
        Some(match s {
            "play" => Kind::Play,
            "buttons" => Kind::Buttons,
            "card" => Kind::Card,
            "server" => Kind::Server,
            "error" => Kind::Error,
            "daynight" => Kind::DayNight,
            _ => return None,
        })
    }

    pub fn length(self) -> f32 {
        match self {
            Kind::Play => 14.0,
            Kind::Buttons => 9.0,
            Kind::Card => 7.0,
            Kind::Server => 13.0,
            Kind::Error => 8.0,
            Kind::DayNight => 16.0,
        }
    }

    /// Mouse keyframes: (time, x, y, button down).
    fn keys(self) -> &'static [(f32, f32, f32, bool)] {
        match self {
            Kind::Play => &[
                (0.0, 330.0, 200.0, false),
                (1.6, 330.0, 200.0, false),
                (2.6, 205.0, 142.0, false), // hover: glow
                (3.6, 205.0, 142.0, true),  // press: 2px down
                (3.9, 205.0, 142.0, false), // release: crack, break, update
                (6.0, 300.0, 120.0, false),
            ],
            Kind::Buttons => &[
                (0.0, 200.0, 120.0, false),
                (1.2, 40.0, 225.0, false), // hover PRISM...
                (2.0, 40.0, 225.0, true),
                (2.25, 40.0, 225.0, false), // click: dissolve to Prism screen
                (3.5, 200.0, 201.0, false), // hover DOWNLOAD (not clicked)
                (4.6, 200.0, 181.0, false), // hover SELECT (not clicked)
                (5.6, 360.0, 60.0, false),
                (6.2, 360.0, 60.0, true),
                (6.4, 360.0, 60.0, false), // click outside: dissolve back
                (8.0, 345.0, 225.0, false), // hover START SERVER
            ],
            Kind::Card => &[
                (0.0, 200.0, 40.0, false),
                (1.0, 20.0, 76.0, false),
                (2.4, 120.0, 76.0, false),
                (3.6, 20.0, 76.0, false),
                (4.8, 120.0, 70.0, false),
                (6.0, 200.0, 40.0, false),
            ],
            Kind::Server => &[(0.0, 200.0, 220.0, false)],
            Kind::Error => &[
                (0.0, 330.0, 200.0, false),
                (1.0, 205.0, 142.0, false),
                (1.6, 205.0, 142.0, true),
                (1.8, 205.0, 142.0, false),
                (4.5, 250.0, 120.0, false),
            ],
            Kind::DayNight => &[
                (0.0, 60.0, 110.0, false),
                (4.0, 340.0, 150.0, false),
                (8.0, 80.0, 170.0, false),
                (12.0, 330.0, 110.0, false),
                (16.0, 200.0, 140.0, false),
            ],
        }
    }

    /// Virtual mouse at time t: eased movement between keyframes.
    pub fn mouse(self, t: f32) -> (f32, f32, bool) {
        let k = self.keys();
        let i = k.iter().rposition(|f| f.0 <= t).unwrap_or(0);
        if i + 1 >= k.len() {
            let f = k[k.len() - 1];
            return (f.1, f.2, f.3);
        }
        let (a, b) = (k[i], k[i + 1]);
        let p = in_out_sine((t - a.0) / (b.0 - a.0));
        (a.1 + (b.1 - a.1) * p, a.2 + (b.2 - a.2) * p, a.3)
    }

    /// Clock override: the day/night demo runs a whole day in 16 seconds.
    pub fn hour(self, t: f32) -> Option<f32> {
        match self {
            Kind::DayNight => Some((5.0 + t * 1.5) % 24.0),
            Kind::Play | Kind::Card | Kind::Buttons => Some(13.0),
            Kind::Server => Some(20.2),
            Kind::Error => Some(17.8),
        }
    }
}

/// A made-up player face: original 8x8 art from a seed (hair, skin, eyes, mouth).
pub fn fake_face(seed: u32) -> [u32; 64] {
    let pal_skin = [0xffe0b48a, 0xffc68a5e, 0xff8d5a3b, 0xfff1c9a5];
    let pal_hair = [0xff2b1d14, 0xff6b3e1f, 0xffd8b24a, 0xff3a3a46, 0xffb0402a];
    let pal_eye = [0xff2b4f9e, 0xff2e7d4f, 0xff5a3a20, 0xff6a2a8a];
    let skin = pal_skin[(seed % 4) as usize];
    let hair = pal_hair[(seed / 4 % 5) as usize];
    let eye = pal_eye[(seed / 3 % 4) as usize];
    let mut f = [skin; 64];
    for x in 0..8 {
        f[x] = hair;
        f[8 + x] = hair;
    }
    f[16] = hair;
    f[23] = hair;
    if seed.is_multiple_of(2) {
        f[24] = hair;
        f[31] = hair;
    }
    f[8 * 4 + 1] = 0xffffffff;
    f[8 * 4 + 2] = eye;
    f[8 * 4 + 5] = eye;
    f[8 * 4 + 6] = 0xffffffff;
    let mouth = 0xff7a3b30;
    f[8 * 6 + 3] = mouth;
    f[8 * 6 + 4] = mouth;
    if seed.is_multiple_of(3) {
        f[8 * 6 + 2] = mouth;
        f[8 * 6 + 5] = mouth;
    }
    f
}
