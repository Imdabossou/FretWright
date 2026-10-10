//! Probabilistic YIN (Mauch & Dixon 2014), ported from librosa.pyin with
//! its default parameters.

use rayon::prelude::*;

use crate::dsp::{self, TINY};

#[derive(Debug, Clone)]
pub struct PyinOutput {
    /// Fundamental frequency per frame (Hz), NaN when unvoiced.
    pub f0: Vec<f64>,
    pub voiced: Vec<bool>,
    /// Probability that the frame is voiced.
    pub voiced_prob: Vec<f64>,
}

const N_THRESHOLDS: usize = 100;
const BETA_A: u32 = 2;
const BETA_B: u32 = 18;
const BOLTZMANN: f64 = 2.0;
const RESOLUTION: f64 = 0.1;
const MAX_TRANSITION_RATE: f64 = 35.92;
const SWITCH_PROB: f64 = 0.01;
const NO_TROUGH_PROB: f64 = 0.01;
const TRANSITION_MIN_PROB: f64 = 1e-4;

/// Regularised incomplete beta I_x(a, b) for integer a, b (closed form).
fn beta_cdf(x: f64, a: u32, b: u32) -> f64 {
    let n = a + b - 1;
    let mut total = 0.0;
    let mut binom = 1.0f64; // C(n, j), built up from C(n, 0)
    for j in 0..=n {
        if j > 0 {
            binom = binom * (n - j + 1) as f64 / j as f64;
        }
        if j >= a {
            total += binom * x.powi(j as i32) * (1.0 - x).powi((n - j) as i32);
        }
    }
    total
}

/// scipy.stats.boltzmann.pmf(k, lambda, N)
fn boltzmann_pmf(k: i64, lambda: f64, n: i64) -> f64 {
    if k < 0 || k >= n {
        return 0.0;
    }
    (1.0 - (-lambda).exp()) * (-lambda * k as f64).exp() / (1.0 - (-lambda * n as f64).exp())
}

/// Cumulative mean normalised difference for one frame, periods min..=max.
fn cmnd(frame: &[f64], min_period: usize, max_period: usize, fwd: &dsp::Fft, inv: &std::sync::Arc<dyn realfft::ComplexToReal<f64>>) -> Vec<f64> {
    let acf = dsp::autocorrelate(frame, max_period + 1, fwd, inv);
    let mut energy = Vec::with_capacity(frame.len());
    let mut acc = 0.0;
    for x in frame {
        acc += x * x;
        energy.push(acc);
    }
    let mut yin = vec![0.0; max_period + 1];
    for k in 1..=max_period {
        yin[k] = 2.0 * (acf[0] - acf[k]) - energy[k - 1];
    }
    let mut cum = 0.0;
    let mut cumulative_mean = vec![0.0; max_period + 1];
    for k in 1..=max_period {
        cum += yin[k];
        cumulative_mean[k] = cum / k as f64;
    }
    (min_period..=max_period).map(|p| yin[p] / (cumulative_mean[p] + TINY)).collect()
}

fn parabolic_shifts(x: &[f64]) -> Vec<f64> {
    let n = x.len();
    let mut s = vec![0.0; n];
    for i in 1..n.saturating_sub(1) {
        let a = x[i + 1] + x[i - 1] - 2.0 * x[i];
        let b = (x[i + 1] - x[i - 1]) / 2.0;
        s[i] = if b.abs() >= a.abs() { 0.0 } else { -b / a };
    }
    s
}

struct FrameObs {
    /// (pitch bin, probability) pairs; later entries overwrite earlier ones.
    bins: Vec<(usize, f64)>,
    voiced_prob: f64,
}

#[allow(clippy::too_many_arguments)]
fn frame_observation(
    yin: &[f64],
    shifts: &[f64],
    thresholds: &[f64],
    beta_probs: &[f64],
    sr: f64,
    min_period: usize,
    fmin: f64,
    n_bins: usize,
    bins_per_semitone: usize,
) -> FrameObs {
    let n = yin.len();
    // local minima (librosa.util.localmin, with the first element compared to the second)
    let mut troughs = Vec::new();
    for i in 0..n {
        let is = if i == 0 {
            n > 1 && yin[0] < yin[1]
        } else if i == n - 1 {
            yin[i] < yin[i - 1]
        } else {
            yin[i] < yin[i - 1] && yin[i] <= yin[i + 1]
        };
        if is {
            troughs.push(i);
        }
    }
    let mut probs_at = vec![0.0; troughs.len()];
    if !troughs.is_empty() {
        let heights: Vec<f64> = troughs.iter().map(|&t| yin[t]).collect();
        let mut n_troughs = vec![0i64; N_THRESHOLDS];
        let mut positions = vec![vec![-1i64; N_THRESHOLDS]; troughs.len()];
        for j in 0..N_THRESHOLDS {
            let mut count = 0i64;
            for (t, &h) in heights.iter().enumerate() {
                if h < thresholds[j + 1] {
                    count += 1;
                }
                positions[t][j] = count - 1;
            }
            n_troughs[j] = count;
        }
        for (t, &h) in heights.iter().enumerate() {
            let mut p = 0.0;
            for j in 0..N_THRESHOLDS {
                if h < thresholds[j + 1] {
                    p += boltzmann_pmf(positions[t][j], BOLTZMANN, n_troughs[j]) * beta_probs[j];
                }
            }
            probs_at[t] = p;
        }
        let global_min = heights
            .iter()
            .enumerate()
            .fold(0, |best, (i, &h)| if h < heights[best] { i } else { best });
        // `!(a < b)` mirrors numpy's `~(a < b)` (true for NaN, unlike `a >= b`)
        #[allow(clippy::neg_cmp_op_on_partial_ord)]
        let below = (0..N_THRESHOLDS).filter(|&j| !(heights[global_min] < thresholds[j + 1])).count();
        probs_at[global_min] += NO_TROUGH_PROB * beta_probs[..below].iter().sum::<f64>();
    }

    let mut bins = Vec::new();
    let mut voiced = 0.0;
    let mut row = vec![0.0; n_bins + 1];
    for (t, &p) in troughs.iter().zip(&probs_at) {
        if p == 0.0 {
            continue;
        }
        let period = (min_period + t) as f64 + shifts[*t];
        let f0 = sr / period;
        let bin = (12.0 * bins_per_semitone as f64 * (f0 / fmin).log2()).round_ties_even();
        let bin = bin.clamp(0.0, n_bins as f64) as usize;
        row[bin] = p; // fancy assignment: the last write wins
        bins.push((bin, p));
    }
    for &v in &row[..n_bins] {
        voiced += v;
    }
    FrameObs { bins, voiced_prob: voiced.clamp(0.0, 1.0) }
}

/// librosa.sequence.transition_local(n, width, window="triangle", wrap=False)
fn transition_local(n: usize, width: usize) -> Vec<Vec<f64>> {
    let w = dsp::triang(width);
    let lpad = (n - width) / 2;
    let mut base = vec![0.0; n];
    base[lpad..lpad + width].copy_from_slice(&w);
    (0..n)
        .map(|i| {
            let shift = n / 2 + i + 1;
            let mut row: Vec<f64> = (0..n).map(|j| base[(j + n - shift % n) % n]).collect();
            for v in row.iter_mut().skip((i + width / 2 + 1).min(n)) {
                *v = 0.0;
            }
            for v in row.iter_mut().take(i.saturating_sub(width / 2)) {
                *v = 0.0;
            }
            let sum: f64 = row.iter().sum();
            row.iter_mut().for_each(|v| *v /= sum);
            row
        })
        .collect()
}

/// Runs pYIN. `y` is mono audio at `sr`.
pub fn pyin(y: &[f64], sr: f64, fmin: f64, fmax: f64, frame_length: usize, hop: usize) -> PyinOutput {
    let padded = dsp::pad_zeros(y, frame_length / 2);
    let n_frames = dsp::frame_count(padded.len(), frame_length, hop);
    let min_period = (sr / fmax).floor() as usize;
    let max_period = ((sr / fmin).ceil() as usize).min(frame_length - 1);

    let thresholds: Vec<f64> = (0..=N_THRESHOLDS).map(|i| i as f64 / N_THRESHOLDS as f64).collect();
    let cdf: Vec<f64> = thresholds.iter().map(|&t| beta_cdf(t, BETA_A, BETA_B)).collect();
    let beta_probs: Vec<f64> = cdf.windows(2).map(|w| w[1] - w[0]).collect();
    let bins_per_semitone = (1.0 / RESOLUTION).ceil() as usize;
    let n_bins = (12.0 * bins_per_semitone as f64 * (fmax / fmin).log2()).floor() as usize + 1;

    // observations per frame, in parallel
    let n_pad = (2 * frame_length - 1).next_power_of_two();
    let obs: Vec<FrameObs> = (0..n_frames)
        .into_par_iter()
        .map_init(
            || (dsp::Fft::new(n_pad), dsp::inverse_plan(n_pad)),
            |(fwd, inv), f| {
                let frame = &padded[f * hop..f * hop + frame_length];
                let yin = cmnd(frame, min_period, max_period, fwd, inv);
                let shifts = parabolic_shifts(&yin);
                frame_observation(&yin, &shifts, &thresholds, &beta_probs, sr, min_period, fmin, n_bins, bins_per_semitone)
            },
        )
        .collect();

    // transition matrix: local triangle for pitch moves, kron with voiced/unvoiced switching
    let max_semitones_per_frame = (MAX_TRANSITION_RATE * 12.0 * hop as f64 / sr).round_ties_even() as usize;
    let width = max_semitones_per_frame * bins_per_semitone + 1;
    let local = transition_local(n_bins, width);
    let n_states = 2 * n_bins;
    let eps = TINY;
    let log_threshold = (TRANSITION_MIN_PROB + eps).ln();
    // predecessors of each state with their log transition probability
    let switch = [[1.0 - SWITCH_PROB, SWITCH_PROB], [SWITCH_PROB, 1.0 - SWITCH_PROB]];
    let preds: Vec<Vec<(u16, f64)>> = (0..n_states)
        .map(|j| {
            let (bj, pj) = (j / n_bins, j % n_bins);
            let mut v = Vec::new();
            for k in 0..n_states {
                let (bk, pk) = (k / n_bins, k % n_bins);
                let lt = (switch[bk][bj] * local[pk][pj] + eps).ln();
                if lt >= log_threshold {
                    v.push((k as u16, lt));
                }
            }
            v
        })
        .collect();

    let log_obs = |o: &FrameObs, state: usize| -> f64 {
        let p = if state < n_bins {
            // last write wins for duplicate bins
            o.bins.iter().rev().find(|(b, _)| *b == state).map_or(0.0, |(_, p)| *p)
        } else {
            (1.0 - o.voiced_prob) / n_bins as f64
        };
        (p + eps).ln()
    };

    let log_init = (1.0 / n_states as f64 + eps).ln();
    let mut value: Vec<f64> = (0..n_states).map(|s| log_obs(&obs[0], s) + log_init).collect();
    let mut ptr = vec![0u16; n_frames * n_states];
    let mut next = vec![0.0; n_states];
    for t in 1..n_frames {
        let o = &obs[t];
        next.par_iter_mut().zip(ptr[t * n_states..(t + 1) * n_states].par_iter_mut()).enumerate().for_each(
            |(j, (nv, pt))| {
                let mut best = f64::NEG_INFINITY;
                let mut arg = 0u16;
                for &(k, lt) in &preds[j] {
                    let c = value[k as usize] + lt;
                    if c > best {
                        best = c;
                        arg = k;
                    }
                }
                *pt = arg;
                *nv = log_obs(o, j) + best;
            },
        );
        std::mem::swap(&mut value, &mut next);
    }
    let mut states = vec![0usize; n_frames];
    if n_frames > 0 {
        let mut s = value
            .iter()
            .enumerate()
            .fold(0, |best, (i, &v)| if v > value[best] { i } else { best });
        states[n_frames - 1] = s;
        for t in (0..n_frames - 1).rev() {
            s = ptr[(t + 1) * n_states + s] as usize;
            states[t] = s;
        }
    }
    let freqs: Vec<f64> = (0..n_bins).map(|k| fmin * 2f64.powf(k as f64 / (12.0 * bins_per_semitone as f64))).collect();
    let voiced: Vec<bool> = states.iter().map(|&s| s < n_bins).collect();
    PyinOutput {
        f0: states.iter().zip(&voiced).map(|(&s, &v)| if v { freqs[s % n_bins] } else { f64::NAN }).collect(),
        voiced,
        voiced_prob: obs.iter().map(|o| o.voiced_prob).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beta_cdf_matches_scipy() {
        // scipy.stats.beta.cdf([0.05, 0.1, 0.3], 2, 18)
        let expected = [0.24529279492938452, 0.5797350211684026, 0.9895781529733731];
        for (x, e) in [0.05, 0.1, 0.3].iter().zip(expected) {
            assert!((beta_cdf(*x, 2, 18) - e).abs() < 1e-9, "{x}");
        }
    }

    #[test]
    fn finds_a_steady_tone() {
        let sr = 22050.0;
        let f = 55.0;
        let y: Vec<f64> = (0..sr as usize).map(|i| (2.0 * std::f64::consts::PI * f * i as f64 / sr).sin() * 0.5).collect();
        let out = pyin(&y, sr, 36.7, 493.9, 4096, 256);
        let mid = out.f0.len() / 2;
        assert!(out.voiced[mid]);
        assert!((out.f0[mid] - f).abs() < 0.5, "{}", out.f0[mid]);
    }
}
