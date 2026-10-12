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
    pub fn spectrum(&self, from: f64, to: f64) -> Option<Vec<f32>> {
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

    /// Salience of every candidate note (`lo..=hi`) in `from..to`.
    pub fn salience_vector(&self, from: f64, to: f64, tuning: f64) -> Option<Vec<f64>> {
        let spec = self.spectrum(from, to)?;
        Some((self.lo..=self.hi).map(|m| self.salience(&spec, m as f64, tuning, BAND_CENTS)).collect())
    }

    /// The note sounding in `from..to` (seconds).
    pub fn estimate(&self, from: f64, to: f64, tuning: f64) -> Option<Estimate> {
        self.estimate_with(from, to, tuning, None)
    }

    /// Like [`estimate`](Self::estimate); `prior` holds one factor per candidate
    /// note (`lo..=hi`) that favours the notes a song actually plays.
    pub fn estimate_with(&self, from: f64, to: f64, tuning: f64, prior: Option<&[f64]>) -> Option<Estimate> {
        let spec = self.spectrum(from, to)?;
        let sal: Vec<f64> = (self.lo..=self.hi).map(|m| self.salience(&spec, m as f64, tuning, BAND_CENTS)).collect();
        let weighted = |i: usize| sal[i] * prior.map_or(1.0, |p| p[i]);
        let best = (0..sal.len()).max_by(|&a, &b| weighted(a).total_cmp(&weighted(b)))?;
        let top = sal[best];
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

/// Features per ranked candidate note.
pub const N_RANK_FEATURES: usize = 16;

/// A candidate note for a stretch of audio, described for the ranker.
#[derive(Debug, Clone, Copy)]
pub struct Candidate {
    pub midi: i32,
    pub features: [f64; N_RANK_FEATURES],
}

impl<'a> Analyzer<'a> {
    /// Strength (square root of the magnitude) of the first six harmonics of `midi`.
    fn harmonic_terms(&self, spec: &[f32], midi: f64, tuning: f64) -> [f64; 6] {
        let hz_per_bin = self.sr / N_FFT as f64;
        let f0 = 440.0 * 2f64.powf((midi + tuning - 69.0) / 12.0);
        let ratio = 2f64.powf(BAND_CENTS / 1200.0) - 1.0;
        std::array::from_fn(|i| {
            let f = f0 * (i + 1) as f64;
            let half = f * ratio;
            let (a, b) = (((f - half) / hz_per_bin).floor().max(1.0) as usize, ((f + half) / hz_per_bin).ceil() as usize);
            if b >= spec.len() { 0.0 } else { (a..=b).map(|k| spec[k] as f64).fold(0.0, f64::max).sqrt() }
        })
    }

    /// The `k` best-matching notes of `spec` with the measurements a ranker needs.
    /// `log_prior[i]` is the log-frequency of note `lo + i` in the song so far.
    pub fn rank_candidates(&self, spec: &[f32], tuning: f64, log_prior: Option<&[f64]>, k: usize) -> Vec<Candidate> {
        let terms: Vec<[f64; 6]> = (self.lo..=self.hi).map(|m| self.harmonic_terms(spec, m as f64, tuning)).collect();
        let sal: Vec<f64> = terms.iter().map(|t| (0..HARMONICS).map(|h| t[h] / ((h + 1) as f64).powf(ALPHA)).sum()).collect();
        let mut order: Vec<usize> = (0..sal.len()).collect();
        order.sort_by(|&a, &b| sal[b].total_cmp(&sal[a]));
        let best = order[0];
        let top = sal[best].max(1e-12);
        let range = (self.hi - self.lo).max(1) as f64;
        order
            .iter()
            .take(k)
            .enumerate()
            .map(|(rank, &i)| {
                let m = self.lo + i as i32;
                let d = m - (self.lo + best as i32);
                let t = &terms[i];
                let odd = t[0] + t[2] + t[4];
                let even = t[1] + t[3];
                let oe = (odd / even.max(1e-9)).min(3.0) / 3.0;
                let below = f64::from(d == -12);
                let reg = i as f64 / range;
                let f = [
                    sal[i] / top,
                    t[0] / top,
                    t[1] / top,
                    t[2] / top,
                    t[3] / top,
                    t[4] / top,
                    1.0 / (1.0 + rank as f64),
                    f64::from(rank == 0),
                    below,
                    f64::from(d == 12),
                    f64::from(matches!(d.abs(), 5 | 7 | 17 | 19)),
                    reg,
                    log_prior.map_or(0.0, |p| p[i] / 5.0),
                    oe,
                    below * reg,
                    below * oe,
                ];
                Candidate { midi: m, features: f }
            })
            .collect()
    }
}
