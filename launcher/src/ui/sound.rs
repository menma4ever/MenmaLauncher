//! Our own sound effects, synthesized at startup (no audio files, nothing from Mojang).
//! Played through Windows' PlaySound from memory, asynchronously.

use std::sync::OnceLock;

#[derive(Clone, Copy)]
pub enum Sfx {
    Click,
    Hover,
    Break,
    Cheer,
    Error,
    Block,
}

const RATE: u32 = 22050;

struct Bank {
    click: Vec<u8>,
    hover: Vec<u8>,
    brk: Vec<u8>,
    cheer: Vec<u8>,
    error: Vec<u8>,
    block: Vec<u8>,
}

static BANK: OnceLock<Bank> = OnceLock::new();

fn wav(samples: &[f32]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut v = Vec::with_capacity(44 + data_len as usize);
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data_len).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes()); // PCM
    v.extend_from_slice(&1u16.to_le_bytes()); // mono
    v.extend_from_slice(&RATE.to_le_bytes());
    v.extend_from_slice(&(RATE * 2).to_le_bytes());
    v.extend_from_slice(&2u16.to_le_bytes());
    v.extend_from_slice(&16u16.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        v.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    v
}

/// Square-wave blip with a pitch slide and a quick fade: the chiptune "tick".
fn blip(f0: f32, f1: f32, secs: f32, vol: f32) -> Vec<f32> {
    let n = (secs * RATE as f32) as usize;
    let mut phase = 0.0f32;
    (0..n)
        .map(|i| {
            let t = i as f32 / n as f32;
            phase += (f0 + (f1 - f0) * t) / RATE as f32;
            let sq = if phase.fract() < 0.5 { 1.0 } else { -1.0 };
            let env = (1.0 - t).powi(2) * (i as f32 / 40.0).min(1.0);
            sq * env * vol
        })
        .collect()
}

fn noise(secs: f32, vol: f32, seed: u32) -> Vec<f32> {
    let n = (secs * RATE as f32) as usize;
    let mut s = seed;
    let mut held = 0.0;
    (0..n)
        .map(|i| {
            if i % 3 == 0 {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                held = (s as f32 / u32::MAX as f32) * 2.0 - 1.0;
            }
            let t = i as f32 / n as f32;
            held * (1.0 - t).powi(3) * vol
        })
        .collect()
}

fn concat(parts: &[Vec<f32>]) -> Vec<f32> {
    parts.iter().flatten().copied().collect()
}

fn mixdown(a: &[f32], b: &[f32]) -> Vec<f32> {
    (0..a.len().max(b.len()))
        .map(|i| a.get(i).unwrap_or(&0.0) + b.get(i).unwrap_or(&0.0))
        .collect()
}

pub fn init() {
    BANK.get_or_init(|| Bank {
        click: wav(&blip(880.0, 520.0, 0.05, 0.22)),
        hover: wav(&blip(1320.0, 1320.0, 0.018, 0.06)),
        brk: wav(&mixdown(&noise(0.22, 0.35, 0x1234_5678), &blip(220.0, 90.0, 0.18, 0.18))),
        cheer: wav(&concat(&[
            blip(523.0, 523.0, 0.09, 0.18),
            blip(659.0, 659.0, 0.09, 0.18),
            blip(784.0, 784.0, 0.09, 0.18),
            blip(1046.0, 1046.0, 0.22, 0.2),
        ])),
        error: wav(&concat(&[blip(330.0, 300.0, 0.14, 0.2), blip(220.0, 180.0, 0.24, 0.2)])),
        block: wav(&blip(660.0, 440.0, 0.03, 0.08)),
    });
}

pub fn play(s: Sfx) {
    let Some(b) = BANK.get() else { return };
    let data = match s {
        Sfx::Click => &b.click,
        Sfx::Hover => &b.hover,
        Sfx::Break => &b.brk,
        Sfx::Cheer => &b.cheer,
        Sfx::Error => &b.error,
        Sfx::Block => &b.block,
    };
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
        // The bank lives for the whole program, so the memory stays valid while playing.
        PlaySoundW(data.as_ptr() as *const u16, std::ptr::null_mut(), SND_MEMORY | SND_ASYNC | SND_NODEFAULT);
    }
    #[cfg(not(windows))]
    let _ = data;
}
