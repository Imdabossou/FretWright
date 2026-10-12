//! Pitch of a stretch of audio whose start and end are known, found from its
//! own spectrum: how well do the harmonics of each candidate note line up
//! with the peaks of the stem?
//!
//! Unlike a frame-by-frame tracker (pYIN) this needs no voicing decision, so
//! it still answers on distorted bass, and it works on short windows, so fast
//! notes are not smeared into their neighbours.

use crate::dsp::{self, Fft};

const N_FFT: usize = 32768;
/// Harmonics summed per candidate.
const HARMONICS: usize = 5;
/// Harmonic h counts 1 / h^ALPHA.
const ALPHA: f64 = 0.5;
/// Half-width of the search around each harmonic, in cents.
const BAND_CENTS: f64 = 25.0;
/// Narrower band used to measure how far off pitch the winner is.
const FINE_BAND_CENTS: f64 = 12.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Estimate {
    pub midi: i32,
    /// How far the winner sits from the equal-tempered note, in semitones (-0.5..0.5).
    pub deviation: f64,
    /// Winner's salience relative to the average candidate (>1; larger = clearer).
    pub confidence: f64,
}

pub struct Analyzer<'a> {
    y: &'a [f64],
    sr: f64,
    fft: Fft,
    lo: i32,
    hi: i32,
}

impl<'a> Analyzer<'a> {
    /// `lo..=hi`: the MIDI notes that can be answered.
    pub fn new(y: &'a [f64], sr: f64, lo: i32, hi: i32) -> Analyzer<'a> {
        Analyzer { y, sr, fft: Fft::new(N_FFT), lo, hi }
    }

    /// Magnitude spectrum of `from..to` (seconds), Hann windowed.
    fn spectrum(&self, from: f64, to: f64) -> Option<Vec<f32>> {
        let (a, b) = ((from * self.sr).max(0.0) as usize, ((to * self.sr) as usize).min(self.y.len()));
        if b < a + 256 {
            return None;
        }
        let w = dsp::hann(b - a);
        let x: Vec<f64> = self.y[a..b].iter().zip(&w).map(|(s, w)| s * w).collect();
        Some(self.fft.power(&x).iter().map(|p| p.sqrt() as f32).collect())
    }

    /// Harmonic salience of the (fractional) note `midi` once the recording's
    /// `tuning` (semitones sharp of A=440) is allowed for.
    fn salience(&self, spec: &[f32], midi: f64, tuning: f64, band_cents: f64) -> f64 {
        let hz_per_bin = self.sr / N_FFT as f64;
        let f0 = 440.0 * 2f64.powf((midi + tuning - 69.0) / 12.0);
        let ratio = 2f64.powf(band_cents / 1200.0) - 1.0;
        (1..=HARMONICS)
            .map(|h| {
                let f = f0 * h as f64;
                let half = f * ratio;
                let (a, b) = (((f - half) / hz_per_bin).floor().max(1.0) as usize, ((f + half) / hz_per_bin).ceil() as usize);
                if b >= spec.len() {
                    return 0.0;
                }
                let peak = (a..=b).map(|i| spec[i] as f64).fold(0.0, f64::max);
                peak.sqrt() / (h as f64).powf(ALPHA)
            })
            .sum()
    }

    /// The note sounding in `from..to` (seconds).
    pub fn estimate(&self, from: f64, to: f64, tuning: f64) -> Option<Estimate> {
        let spec = self.spectrum(from, to)?;
        let sal: Vec<f64> = (self.lo..=self.hi).map(|m| self.salience(&spec, m as f64, tuning, BAND_CENTS)).collect();
        let (best, top) = sal.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map(|(i, v)| (i, *v))?;
        let mean = sal.iter().sum::<f64>() / sal.len() as f64;
        if top <= 0.0 || mean <= 0.0 {
            return None;
        }
        let midi = self.lo + best as i32;
        let deviation = (-5..=5)
            .map(|k| k as f64 * 0.08)
            .max_by(|a, b| {
                let (sa, sb) = (self.salience(&spec, midi as f64 + a, tuning, FINE_BAND_CENTS), self.salience(&spec, midi as f64 + b, tuning, FINE_BAND_CENTS));
                sa.total_cmp(&sb)
            })
            .unwrap_or(0.0);
        Some(Estimate { midi, deviation, confidence: top / mean })
    }
}

/// How far the recording sits from A=440, in semitones (-0.5..0.5): the
/// confidence-weighted circular mean of the winners' deviations.
pub fn tuning_from(estimates: &[Estimate]) -> f64 {
    let (mut sin, mut cos) = (0.0, 0.0);
    for e in estimates {
        let w = (e.confidence - 1.0).max(0.0);
        let angle = 2.0 * std::f64::consts::PI * e.deviation;
        sin += w * angle.sin();
        cos += w * angle.cos();
    }
    if sin == 0.0 && cos == 0.0 { 0.0 } else { sin.atan2(cos) / (2.0 * std::f64::consts::PI) }
}
