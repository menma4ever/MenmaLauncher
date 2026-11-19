//! Easing curves. Nothing on screen moves linearly or jumps instantly.

use std::f32::consts::PI;

pub fn clamp01(t: f32) -> f32 {
    t.clamp(0.0, 1.0)
}

pub fn out_cubic(t: f32) -> f32 {
    let t = clamp01(t);
    1.0 - (1.0 - t).powi(3)
}

pub fn in_out_sine(t: f32) -> f32 {
    let t = clamp01(t);
    -((PI * t).cos() - 1.0) / 2.0
}

pub fn out_back(t: f32) -> f32 {
    let t = clamp01(t);
    let c1 = 1.70158;
    let c3 = c1 + 1.0;
    1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
}

pub fn out_bounce(t: f32) -> f32 {
    let t = clamp01(t);
    let (n1, d1) = (7.5625, 2.75);
    if t < 1.0 / d1 {
        n1 * t * t
    } else if t < 2.0 / d1 {
        let t = t - 1.5 / d1;
        n1 * t * t + 0.75
    } else if t < 2.5 / d1 {
        let t = t - 2.25 / d1;
        n1 * t * t + 0.9375
    } else {
        let t = t - 2.625 / d1;
        n1 * t * t + 0.984375
    }
}

/// Frame-rate independent smooth approach toward a target.
pub fn approach(cur: f32, target: f32, speed: f32, dt: f32) -> f32 {
    cur + (target - cur) * (1.0 - (-speed * dt).exp())
}

/// Slow breathing wave 0..1.
pub fn breathe(t: f32, period: f32) -> f32 {
    in_out_sine(0.5 - 0.5 * (2.0 * PI * t / period).cos())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn curves_hit_endpoints() {
        for f in [out_cubic, in_out_sine, out_back, out_bounce] {
            assert!(f(0.0).abs() < 1e-4);
            assert!((f(1.0) - 1.0).abs() < 1e-4);
        }
        assert!(out_back(0.7) > 1.0, "back overshoots");
    }
}
