//! Onset strength and detection (librosa.onset.onset_strength / onset_detect
//! with their defaults: mel spectral flux, greedy peak picking).

use crate::dsp::{self, TINY};

/// Spectral-flux onset envelope on a log-mel spectrogram.
/// `aggregate_median` selects the median across bands (librosa.beat uses it)
/// instead of the mean.
pub fn onset_strength(y: &[f64], sr: f64, hop: usize, aggregate_median: bool) -> Vec<f64> {
    let n_fft = 2048;
    let spec = dsp::power_spectrogram(y, n_fft, hop);
    let mel = dsp::mel_filters(sr, n_fft, 128, 0.0, sr / 2.0);
    // power_to_db(ref=1, amin=1e-10, top_db=80) over the whole spectrogram
    let mut db: Vec<Vec<f64>> = spec
        .iter()
        .map(|frame| mel.iter().map(|w| 10.0 * w.iter().zip(frame).map(|(a, b)| a * b).sum::<f64>().max(1e-10).log10()).collect())
        .collect();
    let max = db.iter().flatten().copied().fold(f64::NEG_INFINITY, f64::max);
    for v in db.iter_mut().flatten() {
        *v = v.max(max - 80.0);
    }
    let n = db.len();
    let lag = 1;
    let mut env = Vec::with_capacity(n);
    // padding: lag + n_fft / (2 * hop) leading zeros, then trimmed to n frames
    let pad = lag + n_fft / (2 * hop);
    env.extend(std::iter::repeat_n(0.0, pad.min(n)));
    for t in lag..n {
        if env.len() >= n {
            break;
        }
        let mut diffs: Vec<f64> = db[t].iter().zip(&db[t - lag]).map(|(a, b)| (a - b).max(0.0)).collect();
        let v = if aggregate_median {
            diffs.sort_by(|a, b| a.total_cmp(b));
            let m = diffs.len();
            if m % 2 == 1 { diffs[m / 2] } else { (diffs[m / 2 - 1] + diffs[m / 2]) / 2.0 }
        } else {
            diffs.iter().sum::<f64>() / diffs.len() as f64
        };
        env.push(v);
    }
    env.truncate(n);
    env
}

/// Greedy peak picking (librosa.util.peak_pick, method="greedy").
pub fn peak_pick(x: &[f64], pre_max: usize, post_max: usize, pre_avg: usize, post_avg: usize, delta: f64, wait: usize) -> Vec<usize> {
    let n = x.len();
    let mut peaks = Vec::new();
    if n == 0 {
        return peaks;
    }
    let max_of = |a: usize, b: usize| x[a..b].iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let mean_of = |a: usize, b: usize| x[a..b].iter().sum::<f64>() / (b - a) as f64;
    let first = x[0] >= max_of(0, post_max.min(n)) && x[0] >= mean_of(0, post_avg.min(n)) + delta;
    let mut i = if first {
        peaks.push(0);
        wait + 1
    } else {
        1
    };
    while i < n {
        if x[i] != max_of(i.saturating_sub(pre_max), (i + post_max).min(n)) {
            i += 1;
            continue;
        }
        if x[i] < mean_of(i.saturating_sub(pre_avg), (i + post_avg).min(n)) + delta {
            i += 1;
            continue;
        }
        peaks.push(i);
        i += wait + 1;
    }
    peaks
}

/// Onset frames (librosa.onset.onset_detect defaults, backtrack=False).
pub fn onset_detect(y: &[f64], sr: f64, hop: usize) -> Vec<usize> {
    let env = onset_strength(y, sr, hop, false);
    detect_from_envelope(&env, sr, hop)
}

pub fn detect_from_envelope(env: &[f64], sr: f64, hop: usize) -> Vec<usize> {
    let min = env.iter().copied().fold(f64::INFINITY, f64::min);
    let mut norm: Vec<f64> = env.iter().map(|v| v - min).collect();
    let max = norm.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    // negated on purpose: NaN must count as "no onsets", like numpy
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    let empty = !(max > 0.0);
    if empty || !norm.iter().all(|v| v.is_finite()) {
        return Vec::new();
    }
    norm.iter_mut().for_each(|v| *v /= max + TINY);
    // librosa's defaults in frames: floor(seconds * sr / hop), ceil'ed to int
    let frames = |s: f64| (s * sr / hop as f64).floor() as usize;
    peak_pick(&norm, frames(0.03), 1, frames(0.10), frames(0.10) + 1, 0.07, frames(0.03))
}

/// SuperFlux-style onset strength for a bass stem: log-compressed magnitudes
/// over 60-4000 Hz, each frame compared with the (max-filtered) frame `lag`
/// back so vibrato and sustained partials do not count as new attacks.
/// Returns one value per STFT frame at the given hop.
pub fn superflux_strength(y: &[f64], sr: f64, n_fft: usize, hop: usize, lag: usize) -> Vec<f64> {
    const GAMMA: f64 = 10.0;
    let spec = dsp::power_spectrogram(y, n_fft, hop);
    let bin = |f: f64| ((f * n_fft as f64 / sr).round() as usize).clamp(1, n_fft / 2);
    let (lo, hi) = (bin(60.0), bin(4000.0));
    let comp: Vec<Vec<f64>> = spec.iter().map(|fr| fr[lo..=hi].iter().map(|p| (1.0 + GAMMA * p.sqrt()).ln()).collect()).collect();
    let mut env = vec![0.0; comp.len()];
    for t in lag..comp.len() {
        let prev = &comp[t - lag];
        env[t] = comp[t]
            .iter()
            .enumerate()
            .map(|(k, &x)| {
                let m = prev[k.saturating_sub(1)..(k + 2).min(prev.len())].iter().copied().fold(f64::NEG_INFINITY, f64::max);
                (x - m).max(0.0)
            })
            .sum();
    }
    env
}

/// Onset times (seconds) of a bass stem: [`superflux_strength`] peak-picked
/// against a local average. Finds re-picked repeats of one note that the
/// librosa-style detector above misses.
pub fn bass_onsets(y: &[f64], sr: f64) -> Vec<f64> {
    const HOP: usize = 128;
    let delta: f64 = std::env::var("FW_DELTA").ok().and_then(|v| v.parse().ok()).unwrap_or(0.11);
    let env = superflux_strength(y, sr, 1024, HOP, 4);
    // scaled by the 99th percentile, not the maximum: one huge hit must not set the bar for the whole song
    let mut sorted = env.clone();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let scale = sorted.get((sorted.len() as f64 * 0.99) as usize).copied().unwrap_or(0.0);
    if scale <= 0.0 {
        return Vec::new();
    }
    let x: Vec<f64> = env.iter().map(|v| v / scale).collect();
    let frame_s = HOP as f64 / sr;
    let fr = |s: f64| ((s / frame_s).round() as usize).max(1);
    let (win, avg, wait) = (fr(0.03), fr(0.10), fr(0.06));
    let mut out = Vec::new();
    let mut i = 0;
    while i < x.len() {
        let (a, b) = (i.saturating_sub(win), (i + win + 1).min(x.len()));
        let (a2, b2) = (i.saturating_sub(avg), (i + avg + 1).min(x.len()));
        let mean = x[a2..b2].iter().sum::<f64>() / (b2 - a2) as f64;
        if x[a..b].iter().all(|&v| v <= x[i]) && x[i] > mean + delta {
            out.push(i as f64 * frame_s);
            i += wait;
        } else {
            i += 1;
        }
    }
    out
}
