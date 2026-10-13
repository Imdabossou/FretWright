//! Which string was a note played on? A bass string's sound differs a little from its
//! neighbours' at the same pitch (the mix of harmonics, the brightness, how fast it
//! dies away), so a tiny model trained on charted songs can often tell. Its opinion
//! feeds the choice of string and fret (see `fretting`).

use crate::dsp::{self, Fft};

const N_FFT: usize = 32768;
pub const N_FEATURES: usize = 11;
const MAGIC: &[u8; 4] = b"FWST";

pub struct TimbreAnalyzer {
    fft: Fft,
}

impl Default for TimbreAnalyzer {
    fn default() -> Self {
        TimbreAnalyzer { fft: Fft::new(N_FFT) }
    }
}

impl TimbreAnalyzer {
    /// Measurements of the note `midi` sounding in `from..to` seconds of `stem`:
    /// the share of each of its first six harmonics, the spectral centre, slope and
    /// flatness, how much it decays, and the pitch itself. `None` if the span is too short.
    pub fn features(&self, stem: &[f64], sr: f64, from: f64, to: f64, midi: i32) -> Option<[f32; N_FEATURES]> {
        let (a0, b0) = ((from * sr) as usize, (to * sr) as usize);
        if b0 > stem.len() || b0 <= a0 + 2048 {
            return None;
        }
        let hz_per_bin = sr / N_FFT as f64;
        let w = dsp::hann(b0 - a0);
        let x: Vec<f64> = stem[a0..b0].iter().zip(&w).map(|(s, w)| s * w).collect();
        let mag: Vec<f64> = self.fft.power(&x).iter().map(|p| p.sqrt()).collect();
        let f0 = 440.0 * 2f64.powf((midi as f64 - 69.0) / 12.0);
        let peak = |f: f64| -> f64 {
            let half = f * 0.015;
            let (lo, hi) = (((f - half) / hz_per_bin) as usize, ((f + half) / hz_per_bin) as usize + 1);
            if hi >= mag.len() { 0.0 } else { (lo..=hi).map(|k| mag[k]).fold(0.0, f64::max) }
        };
        let hs: Vec<f64> = (1..=6).map(|h| peak(f0 * h as f64)).collect();
        let total = hs.iter().sum::<f64>().max(1e-9);
        let (klo, khi) = ((40.0 / hz_per_bin) as usize, ((3000.0 / hz_per_bin) as usize).min(mag.len()));
        let (mut sw, mut swf) = (0.0, 0.0);
        let (mut sx, mut sy, mut sxx, mut sxy, mut cnt, mut log_sum) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
        for (k, &m) in mag.iter().enumerate().take(khi).skip(klo) {
            let lx = (k as f64 * hz_per_bin).log2();
            let ly = (m + 1e-9).ln();
            sw += m;
            swf += m * lx;
            sx += lx;
            sy += ly;
            sxx += lx * lx;
            sxy += lx * ly;
            cnt += 1.0;
            log_sum += ly;
        }
        let slope = (cnt * sxy - sx * sy) / (cnt * sxx - sx * sx).max(1e-9);
        let flat = (log_sum / cnt).exp() / (sw / cnt).max(1e-9);
        let len = b0 - a0;
        let e = |a: usize, b: usize| (stem[a0 + a..a0 + b].iter().map(|v| v * v).sum::<f64>() / (b - a).max(1) as f64).sqrt();
        let decay = 20.0 * (e(len * 6 / 10, len) / e(0, len * 4 / 10).max(1e-9)).max(1e-6).log10();
        let mut f = [0f32; N_FEATURES];
        for (i, h) in hs.iter().enumerate() {
            f[i] = (h / total) as f32;
        }
        f[6] = (swf / sw.max(1e-9)) as f32;
        f[7] = slope as f32;
        f[8] = decay as f32;
        f[9] = flat as f32;
        f[10] = midi as f32 / 12.0;
        Some(f)
    }
}

/// 11 measurements -> a small hidden layer -> a score for each of the four strings.
#[derive(Debug, Clone)]
pub struct StringNet {
    hidden: usize,
    mu: Vec<f32>,
    sd: Vec<f32>,
    w1: Vec<f32>,
    b1: Vec<f32>,
    w2: Vec<f32>,
    b2: Vec<f32>,
}

impl StringNet {
    pub fn from_bytes(bytes: &[u8]) -> Result<StringNet, String> {
        if bytes.get(..4) != Some(MAGIC) {
            return Err("not a string model file".into());
        }
        let u32_at = |i: usize| bytes.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).ok_or("truncated string model");
        if u32_at(4)? != 1 || u32_at(8)? as usize != N_FEATURES {
            return Err("unsupported string model".into());
        }
        let hidden = u32_at(12)? as usize;
        let mut at = 16;
        let mut take = |n: usize| -> Result<Vec<f32>, String> {
            let b = bytes.get(at..at + 4 * n).ok_or("truncated string model")?;
            at += 4 * n;
            Ok(b.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect())
        };
        Ok(StringNet { hidden, mu: take(N_FEATURES)?, sd: take(N_FEATURES)?, w1: take(hidden * N_FEATURES)?, b1: take(hidden)?, w2: take(4 * hidden)?, b2: take(4)? })
    }

    /// Log-probability of each string for a note with these measurements, given which
    /// strings can play it (`allowed`); disallowed strings get `f32::NEG_INFINITY`.
    pub fn log_probs(&self, f: &[f32; N_FEATURES], allowed: [bool; 4]) -> [f32; 4] {
        let x: Vec<f32> = (0..N_FEATURES).map(|i| (f[i] - self.mu[i]) / self.sd[i]).collect();
        let h: Vec<f32> = (0..self.hidden).map(|j| (self.b1[j] + self.w1[j * N_FEATURES..(j + 1) * N_FEATURES].iter().zip(&x).map(|(a, b)| a * b).sum::<f32>()).max(0.0)).collect();
        let mut z = [f32::NEG_INFINITY; 4];
        for s in 0..4 {
            if allowed[s] {
                z[s] = self.b2[s] + self.w2[s * self.hidden..(s + 1) * self.hidden].iter().zip(&h).map(|(a, b)| a * b).sum::<f32>();
            }
        }
        let max = z.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let sum: f32 = z.iter().filter(|v| v.is_finite()).map(|v| (v - max).exp()).sum();
        z.map(|v| if v.is_finite() { v - max - sum.ln() } else { f32::NEG_INFINITY })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_model_gives_probabilities_over_the_allowed_strings_only() {
        let net = StringNet::from_bytes(include_bytes!("../models/string_net.bin")).expect("model");
        let lp = net.log_probs(&[0.3, 0.5, 0.1, 0.05, 0.03, 0.02, 6.5, -1.2, -5.0, 0.02, 3.0], [false, true, true, false]);
        assert!(lp[0] == f32::NEG_INFINITY && lp[3] == f32::NEG_INFINITY);
        assert!((lp[1].exp() + lp[2].exp() - 1.0).abs() < 1e-5);
        assert!(StringNet::from_bytes(b"nope").is_err());
    }

    #[test]
    fn a_pure_tone_has_its_energy_in_the_first_harmonic() {
        let sr = 22050.0;
        let y: Vec<f64> = (0..22050).map(|i| (2.0 * std::f64::consts::PI * 110.0 * i as f64 / sr).sin()).collect();
        let f = TimbreAnalyzer::default().features(&y, sr, 0.1, 0.8, 45).expect("features");
        assert!(f[0] > 0.9, "first harmonic share {}", f[0]);
        assert!(TimbreAnalyzer::default().features(&y, sr, 0.1, 0.12, 45).is_none());
    }
}
